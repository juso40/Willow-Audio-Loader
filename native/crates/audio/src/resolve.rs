//! Build-independent lookup of the game's statically linked Wwise
//! entries by their decorated names.
//!
//! BL2 and TPS both ship a Wwise symbol table in `.rdata`: a small
//! header, an array of symbol RVAs, an array of name-string RVAs sorted
//! by name, and a `u16` permutation joining them (`names[i]` names
//! `syms[map[i]]`). It maps every `AK::SoundEngine` entry we call to its
//! exact address on whatever build the game loads.
//!
//! On both verified builds the loader stores the table base-relative
//! (RVAs, no base relocations on the region), so the lookup survives any
//! load address. It accepts a relocated table too, detects it from the
//! anchor slot, and applies the base adjustment consistently.
//! Resolution fails closed: without the module anchor, with
//! a header that does not validate, with an ambiguous match or with an
//! unknown name, [`resolve_symbol`] returns `None` and callers keep
//! their stub behavior instead of calling into unknown code.

/// The table's module anchor: the game executable's own name as the
/// table header stores it.
const ANCHOR: &str = "ShippingPC-WillowGame-PCConsole.exe";

/// The second header word on both verified builds (semantics unknown,
/// treated as a validated constant).
const HEADER_VERSION: u32 = 1;

/// Sanity cap for the header's entry counts (both games: 214).
const MAX_ENTRIES: u32 = 4096;

/// Resolves a decorated Wwise symbol name (the full MSVC form, e.g.
/// `?LoadBank@SoundEngine@AK@@YA?AW4AKRESULT@@PBXKAAK@Z`) to its
/// absolute address in the game image.
///
/// `None` off-game and on any image where the symbol table does not
/// resolve cleanly. Callers cache the result (`OnceLock`) and fall back
/// to stub behavior on `None`.
pub fn resolve_symbol(name: &str) -> Option<usize> {
    let (base, image) = game_image()?;
    resolve_in(image, base, name)
}

/// The game executable as `(load address, mapped image bytes)`.
///
/// `None` off Windows, where there is no game image to look at.
#[cfg(windows)]
fn game_image() -> Option<(usize, &'static [u8])> {
    use std::os::raw::c_void;

    unsafe extern "system" {
        fn GetModuleHandleW(module_name: *const u16) -> *mut c_void;
    }

    // SAFETY: a null module name asks for the process executable, which
    // is always mapped while we run inside it.
    let base = unsafe { GetModuleHandleW(std::ptr::null()) } as usize;
    if base == 0 {
        return None;
    }
    // SAFETY: the loader mapped a PE image at `base`. Its headers occupy
    // the first page. This only parses those bytes.
    let headers = unsafe { std::slice::from_raw_parts(base as *const u8, 0x1000) };
    let size = pe_image_size(headers)?;
    if size < 0x1000 {
        return None;
    }
    // SAFETY: a mapped PE image is `SizeOfImage` contiguous readable
    // bytes at its load address for the lifetime of the module.
    let image = unsafe { std::slice::from_raw_parts(base as *const u8, size) };
    Some((base, image))
}

/// The game executable as `(load address, mapped image bytes)`.
///
/// `None` off Windows, where there is no game image to look at.
#[cfg(not(windows))]
fn game_image() -> Option<(usize, &'static [u8])> {
    None
}

/// `SizeOfImage` from a PE32 image's first `0x1000` bytes. `None` when
/// the bytes are not a PE32 image header.
#[cfg(windows)]
fn pe_image_size(headers: &[u8]) -> Option<usize> {
    let read_u16 = |at: usize| {
        let bytes = headers.get(at..at + 2)?;
        Some(u16::from_le_bytes(bytes.try_into().expect("2 bytes")) as usize)
    };
    let read_u32 = |at: usize| {
        let bytes = headers.get(at..at + 4)?;
        Some(u32::from_le_bytes(bytes.try_into().expect("4 bytes")) as usize)
    };
    if read_u16(0)? != 0x5a4d {
        return None; // "MZ"
    }
    let nt = read_u32(0x3c)?;
    if headers.get(nt..nt + 4)? != [b'P', b'E', 0, 0] {
        return None;
    }
    if read_u16(nt + 24)? != 0x10b {
        return None; // PE32 only (the games are 32-bit)
    }
    read_u32(nt + 24 + 56)
}

/// The symbol table's three arrays inside a mapped game image (offsets
/// into `image`, which is also RVA space).
struct Table {
    /// The image's load address, for converting stored values.
    base: usize,
    /// Whether the table stores relocated absolute addresses instead of
    /// RVAs (detected from the anchor slot, both games store RVAs).
    relocated: bool,
    /// Offset of the `u32` symbol array.
    syms: usize,
    /// Offset of the `u32` name-string array.
    names: usize,
    /// Offset of the `u16` name-to-symbol index map.
    map: usize,
    /// Entries per array (all three carry `count` entries).
    count: usize,
}

impl Table {
    /// Interprets a stored array value as an RVA.
    fn value_rva(&self, stored: u32) -> Option<usize> {
        if self.relocated {
            (stored as usize).checked_sub(self.base)
        } else {
            Some(stored as usize)
        }
    }
}

/// Performs the symbol-table lookup for [`resolve_symbol`], using
/// the image bytes and load address supplied by [`game_image`].
fn resolve_in(image: &[u8], base: usize, name: &str) -> Option<usize> {
    let table = locate_table(image, base)?;
    let name_rva = unique_rva(image, name.as_bytes())?;

    // The name array names each symbol exactly once (its map is a
    // permutation), so the index lookup is exact.
    let mut found = None;
    for index in 0..table.count {
        let stored = read_u32(image, table.names + 4 * index)?;
        if table.value_rva(stored) == Some(name_rva) {
            if found.is_some() {
                return None; // ambiguous
            }
            found = Some(index);
        }
    }
    let index = found?;
    let sym_index = read_u16(image, table.map + 2 * index)? as usize;
    if sym_index >= table.count {
        return None;
    }
    let sym_rva = table.value_rva(read_u32(image, table.syms + 4 * sym_index)?)?;
    if sym_rva == 0 || sym_rva >= image.len() {
        return None;
    }
    Some(base + sym_rva)
}

/// Finds and validates the table header. The anchor string's pointer
/// slot is the second header word, so every header candidate is a `u32`
/// equal to the anchor's RVA (or its relocated address). The validated
/// shape around the slot rejects decoys.
fn locate_table(image: &[u8], base: usize) -> Option<Table> {
    let anchor_rva = unique_rva(image, ANCHOR.as_bytes())?;
    for relocated in [false, true] {
        let key = if relocated {
            base.wrapping_add(anchor_rva) as u32
        } else {
            anchor_rva as u32
        };
        for slot in matching_u32s(image, key) {
            if let Some(table) = read_header(image, base, relocated, slot) {
                return Some(table);
            }
        }
    }
    None
}

/// The table header whose anchor slot sits at `slot`, validated.
///
/// Header shape around the slot: `{0, anchor_rva, 1, count, count,
/// syms_rva, names_rva, map_rva}`. The reader checks every field before
/// it trusts the arrays: the two counts must agree and be plausible, and
/// the three arrays must be aligned and lie inside the image with room
/// for all entries.
fn read_header(image: &[u8], base: usize, relocated: bool, slot: usize) -> Option<Table> {
    if slot < 4 || read_u32(image, slot - 4)? != 0 {
        return None;
    }
    if read_u32(image, slot + 4)? != HEADER_VERSION {
        return None;
    }
    let count = read_u32(image, slot + 8)? as usize;
    if count == 0 || count > MAX_ENTRIES as usize || read_u32(image, slot + 12)? as usize != count {
        return None;
    }
    let to_rva = |stored: u32| {
        if relocated {
            (stored as usize).checked_sub(base)
        } else {
            Some(stored as usize)
        }
    };
    let table = Table {
        base,
        relocated,
        syms: to_rva(read_u32(image, slot + 16)?)?,
        names: to_rva(read_u32(image, slot + 20)?)?,
        map: to_rva(read_u32(image, slot + 24)?)?,
        count,
    };
    // The arrays must be 2-aligned and lie inside the image with room
    // for all entries.
    let fits = |at: usize, span: usize| {
        at.is_multiple_of(2) && at.checked_add(span).is_some_and(|end| end <= image.len())
    };
    if !fits(table.syms, 4 * count) || !fits(table.names, 4 * count) || !fits(table.map, 2 * count)
    {
        return None;
    }
    Some(table)
}

/// The RVA of `needle` in `image`, `NUL`-terminated. `None` when absent
/// **or ambiguous**: a stale name must not silently pick a winner.
fn unique_rva(image: &[u8], needle: &[u8]) -> Option<usize> {
    let mut with_nul = Vec::with_capacity(needle.len() + 1);
    with_nul.extend_from_slice(needle);
    with_nul.push(0);
    let matches = find_all(image, &with_nul);
    if matches.len() != 1 {
        return None;
    }
    matches[0].into()
}

/// Every offset in `image` whose little-endian `u32` equals `value`.
/// Array slots are 4-aligned by construction (arrays of aligned RVAs),
/// so the scan steps by 4.
fn matching_u32s(image: &[u8], value: u32) -> Vec<usize> {
    (0..image.len().saturating_sub(3))
        .step_by(4)
        .filter(|&at| read_u32(image, at) == Some(value))
        .collect()
}

/// Every offset of `needle` in `haystack` (first two, uniqueness is all
/// callers check).
fn find_all(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    let mut hits = Vec::new();
    if needle.is_empty() || needle.len() > haystack.len() {
        return hits;
    }
    let last = haystack.len() - needle.len();
    let mut at = 0;
    while at <= last {
        match haystack[at..=last]
            .iter()
            .position(|&byte| byte == needle[0])
        {
            Some(offset) => at += offset,
            None => break,
        }
        if at > last {
            break;
        }
        if &haystack[at..at + needle.len()] == needle {
            hits.push(at);
            if hits.len() == 2 {
                break;
            }
        }
        at += 1;
    }
    hits
}

/// The little-endian `u32` at `at`, `None` when out of bounds.
fn read_u32(image: &[u8], at: usize) -> Option<u32> {
    let bytes = image.get(at..at + 4)?;
    Some(u32::from_le_bytes(bytes.try_into().expect("4 bytes")))
}

/// The little-endian `u16` at `at`, `None` when out of bounds.
fn read_u16(image: &[u8], at: usize) -> Option<u16> {
    let bytes = image.get(at..at + 2)?;
    Some(u16::from_le_bytes(bytes.try_into().expect("2 bytes")))
}
