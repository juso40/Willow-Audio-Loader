//! Direct calls into the game's statically linked Wwise sound engine.
//!
//! The shipped script surface deliberately cannot load banks
//! (`AkLoadBank (Nonfunctional)` in `AkAudio.upk`), but the native
//! `AK::SoundEngine` entry points are intact. Each entry resolves at
//! first use by its decorated name through the symbol table BL2 and TPS
//! both ship ([`crate::resolve`]), so one build of this crate
//! serves both games and any build carrying the table.
//!
//! - `AK::SoundEngine::LoadBank(const void*, unsigned long, AkBankID&)`
//!   (`?LoadBank@...@@PBXKAAK@Z`) **loads a bank from memory**, no file
//!   discovery involved. BL2 `0x012e24e0`, TPS `0x011d35a0`.
//! - `AK::SoundEngine::UnloadBank(AkBankID, long*)`
//!   (`?UnloadBank@...@@KPAJ@Z`). BL2 `0x012e2620`, TPS `0x011d36e0`.
//! - `AK::SoundEngine::PostEvent(AkUniqueID, AkGameObjectID, ...)`
//!   (`?PostEvent@...@@YAKKIKP6AX...K@Z`) returns the playing id (0 =
//!   the engine could not resolve the event). BL2 `0x012e5590`, TPS
//!   `0x011d6640`.
//! - `AK::SoundEngine::RegisterGameObj(AkGameObjectID)`
//!   (`?RegisterGameObj@...@@I@Z`), `UnregisterGameObj`
//!   (`?UnregisterGameObj@...@@I@Z`), `SetActiveListeners`
//!   (`?SetActiveListeners@...@@IK@Z`). BL2 `0x012e2100`/`0x012e2150`/
//!   `0x012e1800`, TPS `0x011d31c0`/`0x011d3210`/`0x011d28c0`.
//! - `AK::SoundEngine::GetIDFromString(const char*)`
//!   (`?GetIDFromString@...@@YAKPBD@Z`). BL2 `0x012e2d70`, TPS
//!   `0x011d3e30`.
//!
//! All entry points listed above use the `__cdecl` calling convention.
//! Functions returning `AKRESULT` return a signed 32-bit result code:
//! `1` (`AK_Success`) indicates success. Our load/unload wrappers include
//! the numeric result code in error messages when a call fails.
//! The bank loader returns code `7` for corrupt bank data.
//! `AkGameObjectID` is `u32` in this Wwise version.
//!
//! We keep an aligned copy of each loaded bank's bytes in a registry
//! until `unload_bank`. This Wwise version may retain a pointer to the
//! supplied bytes rather than copy them, so the buffer must remain valid
//! while the bank is loaded.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::error::{AudioError, Result};

/// A bank buffer aligned to 16 bytes.
///
/// `LoadBank`'s prologue rejects anything misaligned with
/// `(ptr & 0xf) == 0` before parsing (error `0x1F`), while a `Vec<u8>`
/// only guarantees 1-byte alignment. The engine may keep reading the
/// buffer (embedded media is consumed lazily), so the allocation lives
/// until `unload_bank`.
struct AlignedBank {
    ptr: *mut u8,
    len: usize,
}

// SAFETY: plain `u8` bytes shared as data only. The engine reads them.
unsafe impl Send for AlignedBank {}

impl AlignedBank {
    /// Copies `data` into a fresh 16-byte-aligned allocation.
    fn new(data: &[u8]) -> Option<Self> {
        let len = data.len();
        let layout = std::alloc::Layout::from_size_align(len, 16).ok()?;
        if layout.size() == 0 {
            return None;
        }
        // SAFETY: layout is nonzero-sized. The allocation is freed in `Drop`.
        let ptr = unsafe { std::alloc::alloc(layout) };
        if ptr.is_null() {
            return None;
        }
        // SAFETY: `ptr` owns `len` fresh bytes.
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, len) };
        Some(Self { ptr, len })
    }

    fn as_ptr(&self) -> *const u8 {
        self.ptr
    }

    fn size(&self) -> usize {
        self.len
    }

    /// The bank bytes as a slice.
    fn bytes(&self) -> &[u8] {
        // SAFETY: `ptr` owns `len` initialized bytes for the struct's life.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl Drop for AlignedBank {
    fn drop(&mut self) {
        let layout = std::alloc::Layout::from_size_align(self.len, 16).expect("layout was valid");
        // SAFETY: `ptr` came from `alloc` with the same layout.
        unsafe { std::alloc::dealloc(self.ptr, layout) };
    }
}

/// Loaded banks' aligned buffers, keyed by bank id. Keeps memory-view
/// banks valid (and 16-byte aligned) until unload.
fn loaded() -> &'static Mutex<HashMap<u32, AlignedBank>> {
    static LOADED: OnceLock<Mutex<HashMap<u32, AlignedBank>>> = OnceLock::new();
    LOADED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Loads a bank (the bytes [`crate::craft::craft_bank`] builds, or a
/// stock `.bnk`) into Wwise and returns its bank id.
///
/// Repeat loads are idempotent: the engine rejects a bank id it already
/// holds, so the registry short-circuits (the same id with different
/// bytes is an error). Errors when Wwise rejects the data (`AKRESULT`
/// forwarded).
pub fn load_bank(data: Vec<u8>) -> Result<u32> {
    if data.len() < 24 || &data[..4] != b"BKHD" {
        return Err(AudioError::format(format!(
            "not a bank: leading {:02x?} (expected BKHD)",
            &data[..data.len().min(4)]
        )));
    }
    // Repeat loads are idempotent: the engine rejects a bank id it
    // already holds, so short-circuit on the id from `BKHD` (+0xC, the
    // same field the engine's own out-param reads).
    let file_id = u32::from_le_bytes(data[0xC..0x10].try_into().expect("4 bytes"));
    {
        let registry = loaded().lock().expect("bank registry poisoned");
        if let Some(existing) = registry.get(&file_id) {
            return if existing.bytes() == data.as_slice() {
                Ok(file_id)
            } else {
                Err(AudioError::format(format!(
                    "bank id 0x{file_id:08x} is already loaded with different content"
                )))
            };
        }
    }
    let mut bank_id: u32 = 0;
    let bank = AlignedBank::new(&data)
        .ok_or_else(|| AudioError::format("cannot allocate an aligned bank buffer"))?;
    let result = call_load_bank(bank.as_ptr(), bank.size() as u32, &mut bank_id);
    if result != 1 {
        return Err(AudioError::format(format!(
            "AK::SoundEngine::LoadBank rejected the bank (AKRESULT {result})"
        )));
    }
    loaded()
        .lock()
        .expect("bank registry poisoned")
        .insert(bank_id, bank);
    Ok(bank_id)
}

/// Unloads a bank loaded through [`load_bank`], releasing its bytes.
/// `Ok(false)` when the id is not one of ours (game banks are the game's
/// business).
pub fn unload_bank(bank_id: u32) -> Result<bool> {
    let mut released: i32 = 0;
    let result = call_unload_bank(bank_id, &mut released);
    let ours = loaded()
        .lock()
        .expect("bank registry poisoned")
        .remove(&bank_id)
        .is_some();
    if result != 1 {
        return Err(AudioError::format(format!(
            "AK::SoundEngine::UnloadBank failed (AKRESULT {result})"
        )));
    }
    Ok(ours)
}

/// Posts a Wwise event id on a game object, returning the playing id.
///
/// The id overload of `AK::SoundEngine::PostEvent` (`0x012e5590`) with
/// no callback and no external sources. **Returns 0 when the engine
/// cannot resolve the event id** (bank not loaded / no such event).
/// Useful for to check whether the engine knows the provided eventId or not.
/// Furthermore, it bypasses `AkEvent` objects and everything Unreal.
/// The game object should be registered (and listener-bound) first, see
/// [`register_game_object`] and [`set_active_listeners`].
pub fn post_event(event_id: u32, game_object: u32) -> u32 {
    call_post_event(event_id, game_object)
}

/// Registers a game object id with the sound engine. `true` on success.
pub fn register_game_object(game_object: u32) -> bool {
    call_register_game_object(game_object) == 1
}

/// Unregisters a game object id. `true` on success.
pub fn unregister_game_object(game_object: u32) -> bool {
    call_unregister_game_object(game_object) == 1
}

/// Binds listener bits (`mask`, bit 0 = listener 0) to a game object.
/// Without this, sounds posted on a fresh game object have no listener
/// and can come out silent. `true` on success.
pub fn set_active_listeners(game_object: u32, mask: u32) -> bool {
    call_set_active_listeners(game_object, mask) == 1
}

/// The Wwise id of an event name (`GetIDFromString`): the hash the game
/// itself uses when resolving names to ids. 0 for an empty name.
pub fn get_id_from_string(name: &str) -> u32 {
    let Ok(name) = std::ffi::CString::new(name) else {
        return 0;
    };
    call_get_id_from_string(name.as_ptr())
}

fn call_post_event(event_id: u32, game_object: u32) -> u32 {
    // `?PostEvent@SoundEngine@AK@@YAKKIKP6AXW4AkCallbackType@@
    // PAUAkCallbackInfo@@@ZPAXKPAUAkExternalSourceInfo@@K@Z`
    // (`AkPlayingID __cdecl PostEvent(AkUniqueID, AkGameObjectID,
    // ulong, Callback, void*, ulong, AkExternalSourceInfo*, ulong)`,
    // the id overload).
    const NAME: &str = "?PostEvent@SoundEngine@AK@@YAKKIKP6AXW4AkCallbackType@@PAUAkCallbackInfo@@@ZPAXKPAUAkExternalSourceInfo@@K@Z";
    type Entry = extern "C" fn(u32, u32, u32, u32, u32, u32, u32, u32) -> u32;
    static ENTRY: OnceLock<Option<Entry>> = OnceLock::new();
    let entry = *ENTRY.get_or_init(|| {
        let addr = crate::resolve::resolve_symbol(NAME)?;
        // SAFETY: `addr` is the entry the game's symbol table names
        // `NAME`. `Entry` is its recorded `__cdecl` signature, called with null
        // callback/external-source pointers per the engine's own caller
        // `FUN_00ee7460`.
        Some(unsafe { std::mem::transmute::<usize, Entry>(addr) })
    });
    match entry {
        Some(post) => post(event_id, game_object, 0, 0, 0, 0, 0, 0),
        // Unresolvable: 0.
        None => 0,
    }
}

fn call_register_game_object(game_object: u32) -> i32 {
    // `?RegisterGameObj@SoundEngine@AK@@YA?AW4AKRESULT@@I@Z`
    // (`AKRESULT __cdecl RegisterGameObj(AkGameObjectID)`, `u32` id).
    const NAME: &str = "?RegisterGameObj@SoundEngine@AK@@YA?AW4AKRESULT@@I@Z";
    type Entry = extern "C" fn(u32) -> i32;
    static ENTRY: OnceLock<Option<Entry>> = OnceLock::new();
    let entry = *ENTRY.get_or_init(|| {
        let addr = crate::resolve::resolve_symbol(NAME)?;
        // SAFETY: `addr` is the entry the game's symbol table names
        // `NAME`. `Entry` is its recorded `__cdecl` signature.
        Some(unsafe { std::mem::transmute::<usize, Entry>(addr) })
    });
    match entry {
        Some(register) => register(game_object),
        None => -1, // no entry: `AK_NotInitialized`
    }
}

fn call_unregister_game_object(game_object: u32) -> i32 {
    // `?UnregisterGameObj@SoundEngine@AK@@YA?AW4AKRESULT@@I@Z`
    // (`AKRESULT __cdecl UnregisterGameObj(AkGameObjectID)`).
    const NAME: &str = "?UnregisterGameObj@SoundEngine@AK@@YA?AW4AKRESULT@@I@Z";
    type Entry = extern "C" fn(u32) -> i32;
    static ENTRY: OnceLock<Option<Entry>> = OnceLock::new();
    let entry = *ENTRY.get_or_init(|| {
        let addr = crate::resolve::resolve_symbol(NAME)?;
        // SAFETY: `addr` is the entry the game's symbol table names
        // `NAME`. `Entry` is its recorded `__cdecl` signature.
        Some(unsafe { std::mem::transmute::<usize, Entry>(addr) })
    });
    match entry {
        Some(unregister) => unregister(game_object),
        None => -1, // no entry: `AK_NotInitialized`
    }
}

fn call_set_active_listeners(game_object: u32, mask: u32) -> i32 {
    // `?SetActiveListeners@SoundEngine@AK@@YA?AW4AKRESULT@@IK@Z`
    // (`AKRESULT __cdecl SetActiveListeners(AkGameObjectID, ulong)`,
    // listener bits in `mask`).
    const NAME: &str = "?SetActiveListeners@SoundEngine@AK@@YA?AW4AKRESULT@@IK@Z";
    type Entry = extern "C" fn(u32, u32) -> i32;
    static ENTRY: OnceLock<Option<Entry>> = OnceLock::new();
    let entry = *ENTRY.get_or_init(|| {
        let addr = crate::resolve::resolve_symbol(NAME)?;
        // SAFETY: `addr` is the entry the game's symbol table names
        // `NAME`. `Entry` is its recorded `__cdecl` signature.
        Some(unsafe { std::mem::transmute::<usize, Entry>(addr) })
    });
    match entry {
        Some(set) => set(game_object, mask),
        None => -1, // no entry: `AK_NotInitialized`
    }
}

fn call_get_id_from_string(name: *const std::os::raw::c_char) -> u32 {
    // `?GetIDFromString@SoundEngine@AK@@YAKPBD@Z`
    // (`AkUniqueID __cdecl GetIDFromString(const char*)`, the narrow
    // overload).
    const NAME: &str = "?GetIDFromString@SoundEngine@AK@@YAKPBD@Z";
    type Entry = extern "C" fn(*const std::os::raw::c_char) -> u32;
    static ENTRY: OnceLock<Option<Entry>> = OnceLock::new();
    let entry = *ENTRY.get_or_init(|| {
        let addr = crate::resolve::resolve_symbol(NAME)?;
        // SAFETY: `addr` is the entry the game's symbol table names
        // `NAME`. `Entry` is its recorded `__cdecl` signature,
        // and `name` is a live NUL-terminated C string for the call.
        Some(unsafe { std::mem::transmute::<usize, Entry>(addr) })
    });
    match entry {
        Some(hash) => hash(name),
        None => 0, // no entry: 0
    }
}

fn call_load_bank(ptr: *const u8, size: u32, out_bank_id: *mut u32) -> i32 {
    // `?LoadBank@SoundEngine@AK@@YA?AW4AKRESULT@@PBXKAAK@Z`
    // (`AKRESULT __cdecl LoadBank(const void*, unsigned long,
    // AkBankID&)`, the memory overload).
    const NAME: &str = "?LoadBank@SoundEngine@AK@@YA?AW4AKRESULT@@PBXKAAK@Z";
    type Entry = extern "C" fn(*const u8, u32, *mut u32) -> i32;
    static ENTRY: OnceLock<Option<Entry>> = OnceLock::new();
    let entry = *ENTRY.get_or_init(|| {
        let addr = crate::resolve::resolve_symbol(NAME)?;
        // SAFETY: `addr` is the entry the game's symbol table names
        // `NAME`. `Entry` is its recorded `__cdecl` signature
        // . `ptr`/`size` describe a
        // valid allocation owned by the caller for the duration of the
        // call. `out_bank_id` points at a live local.
        Some(unsafe { std::mem::transmute::<usize, Entry>(addr) })
    });
    match entry {
        Some(load) => load(ptr, size, out_bank_id),
        // Unresolvable: `AK_NotInitialized`
        None => -1,
    }
}

fn call_unload_bank(bank_id: u32, out_released: *mut i32) -> i32 {
    // `?UnloadBank@SoundEngine@AK@@YA?AW4AKRESULT@@KPAJ@Z`
    // (`AKRESULT __cdecl UnloadBank(AkBankID, long*)`).
    const NAME: &str = "?UnloadBank@SoundEngine@AK@@YA?AW4AKRESULT@@KPAJ@Z";
    type Entry = extern "C" fn(u32, *mut i32) -> i32;
    static ENTRY: OnceLock<Option<Entry>> = OnceLock::new();
    let entry = *ENTRY.get_or_init(|| {
        let addr = crate::resolve::resolve_symbol(NAME)?;
        // SAFETY: `addr` is the entry the game's symbol table names
        // `NAME`. `Entry` is its recorded `__cdecl` signature, and `out_released`
        // points at a live local for the call.
        Some(unsafe { std::mem::transmute::<usize, Entry>(addr) })
    });
    match entry {
        Some(unload) => unload(bank_id, out_released),
        None => -1, // no entry: `AK_NotInitialized`
    }
}
