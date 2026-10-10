//! The Python-facing surface of the `blaudio._blaudio` module.
//!
//! Three groups:
//!
//! - **The add path**: `craft_bank` (build a bank from any audio),
//!   `to_wem` (normalize audio to `.wem`), `load_bank`/`unload_bank`
//!   (`AK::SoundEngine`, entries resolve by name through `audio::resolve`).
//! - **Archive tools**: `media_list`/`extract_media`/`pck_probe`
//!   (inventory and read `.pck` payloads without modifying files).
//! - **Raw primitives** for diagnosis: `post_event`,
//!   `register_game_object`, `set_active_listeners`,
//!   `get_id_from_string`.
//!
//! Everything runs on the calling thread (the game thread, where
//! PythonSDK executes mods). The archive tools are pure file-format work
//! and run outside the game too, the engine calls error off-game.
//! Event objects and playing live in the Python
//! package (`blaudio.api`, `blaudio.objects`, `blaudio.diagnose`).
//!
//! Consumer: the `blaudio` crate (module `blaudio._blaudio`).

use std::path::PathBuf;

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3_stub_gen::derive::gen_stub_pyfunction;

/// [`audio::AudioError`] to Python: [`PyRuntimeError`] with the full
/// message (paths, hex ids and offsets included).
fn audio_error(e: audio::AudioError) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}

/// Normalizes any audio to a `.wem` the bank crafter embeds.
///
/// A Wwise `.wem` (Vorbis, PCM or ADPCM) passes through untouched.
/// Anything else (WAV, MP3, FLAC, OGG, AAC, ...) is decoded to PCM and
/// wrapped in a PCM `.wem`. The output keeps the source channels and
/// sample rate.
/// Positioning is a bank setting, independent of the media channels.
#[gen_stub_pyfunction]
#[pyfunction]
pub fn to_wem(data: Vec<u8>) -> PyResult<Vec<u8>> {
    audio::decode::to_wem(&data).map_err(audio_error)
}

/// Every addressable media id in an archive (streams and bank-embedded
/// media) with sizes: `[(media_id, size), ...]`, streams first.
#[gen_stub_pyfunction]
#[pyfunction]
pub fn media_list(pck_path: &str) -> PyResult<Vec<(u32, u32)>> {
    let pck = audio::pck::Pck::open(std::path::Path::new(pck_path)).map_err(audio_error)?;
    Ok(pck.list_media())
}

/// Reads one media payload (`.wem`) from an archive: a stream by media
/// id, or bank-embedded media through `DIDX`.
#[gen_stub_pyfunction]
#[pyfunction]
pub fn extract_media(pck_path: &str, media_id: u32) -> PyResult<Vec<u8>> {
    let pck = audio::pck::Pck::open(std::path::Path::new(pck_path)).map_err(audio_error)?;
    pck.extract_media(media_id).map_err(audio_error)
}

/// Posts a raw Wwise event id on a game object and returns the playing id.
///
/// Direct `AK::SoundEngine::PostEvent`, bypassing `AkEvent` objects and
/// everything Unreal. Returns 0 when the engine cannot resolve the id
/// (bank not loaded / no such event). Register and listener-bind the
/// game object first (`register_game_object`, `set_active_listeners`).
#[gen_stub_pyfunction]
#[pyfunction]
pub fn post_event(event_id: u32, game_object: u32) -> u32 {
    audio::wwise::post_event(event_id, game_object)
}

/// Registers a game object id with the sound engine. `True` on success.
#[gen_stub_pyfunction]
#[pyfunction]
pub fn register_game_object(game_object: u32) -> bool {
    audio::wwise::register_game_object(game_object)
}

/// Unregisters a game object id. `True` on success.
#[gen_stub_pyfunction]
#[pyfunction]
pub fn unregister_game_object(game_object: u32) -> bool {
    audio::wwise::unregister_game_object(game_object)
}

/// Binds listener bits (`mask`, bit 0 = listener 0) to a game object.
/// Sounds posted on an unbound game object can be silent.
#[gen_stub_pyfunction]
#[pyfunction]
pub fn set_active_listeners(game_object: u32, mask: u32) -> bool {
    audio::wwise::set_active_listeners(game_object, mask)
}

/// The Wwise id of an event name (the engine's own `GetIDFromString`
/// hash). 0 for an empty name or an interior NUL.
#[gen_stub_pyfunction]
#[pyfunction]
pub fn get_id_from_string(name: &str) -> u32 {
    audio::wwise::get_id_from_string(name)
}

/// Builds a minimal bank with brand-new sounds and returns the bytes.
///
/// One Wwise event per entry: `event_ids[i]` plays `wems[i]`. Each entry
/// is any common audio format (WAV, MP3, FLAC, OGG, AAC, ...) or a
/// `.wem`. Non-`.wem` input is decoded to PCM. `bank_name` names the
/// bank in its `STID` entry (default `BlAudio_<bank_id>`). Feed the
/// result to `load_bank`, then post any event id to hear it. Raises when
/// ids would collide through the internal derivation (pick
/// well-separated event ids). `positioning` optionally supplies one
/// mode per event: `2d`, `2d_panned`, `3d`, `3d_static`,
/// `3d_no_attenuation`, `3d_unspatialized`, or `weapon`. Default `3d`
/// preserves the donor behavior. `weapon` selects first-person 2D and
/// third-person 3D using the game's switch. See docs/positioning.md.
#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (bank_id, bank_name, event_ids, wems, positioning = None))]
pub fn craft_bank(
    bank_id: u32,
    bank_name: Option<String>,
    event_ids: Vec<u32>,
    wems: Vec<Vec<u8>>,
    positioning: Option<Vec<String>>,
) -> PyResult<Vec<u8>> {
    if event_ids.len() != wems.len() {
        return Err(PyValueError::new_err(
            "event_ids and wems must have the same length",
        ));
    }
    let modes = match positioning {
        None => vec![audio::craft::Positioning::Spatial; event_ids.len()],
        Some(values) => {
            if values.len() != event_ids.len() {
                return Err(PyValueError::new_err(
                    "positioning and event_ids must have the same length",
                ));
            }
            values
                .iter()
                .map(|value| {
                    value
                        .parse::<audio::craft::Positioning>()
                        .map_err(|error| PyValueError::new_err(error.to_string()))
                })
                .collect::<PyResult<Vec<_>>>()?
        }
    };
    let sounds: Vec<audio::craft::NewSound> = event_ids
        .into_iter()
        .zip(wems)
        .zip(modes)
        .map(|((event_id, media), positioning)| audio::craft::NewSound {
            event_id,
            media,
            size_hint: None,
            positioning,
        })
        .collect();
    let name = bank_name.unwrap_or_else(|| format!("BlAudio_{bank_id:08X}"));
    audio::craft::craft_bank(bank_id, &name, &sounds).map_err(audio_error)
}

/// Loads a bank into Wwise from memory and returns its bank id.
///
/// Takes the bytes from `craft_bank` or a stock `.bnk` file. This is the
/// entry the game's script surface lacks (`AkLoadBank (Nonfunctional)`):
/// it calls `AK::SoundEngine::LoadBank` on the game thread. Raises when
/// Wwise rejects the data or off-game.
#[gen_stub_pyfunction]
#[pyfunction]
pub fn load_bank(data: Vec<u8>) -> PyResult<u32> {
    audio::wwise::load_bank(data).map_err(audio_error)
}

/// Unloads a bank loaded through `load_bank`, releasing its bytes.
/// Returns whether the id was one of ours (we leave game banks alone).
#[gen_stub_pyfunction]
#[pyfunction]
pub fn unload_bank(bank_id: u32) -> PyResult<bool> {
    audio::wwise::unload_bank(bank_id).map_err(audio_error)
}

/// Dumps a Wwise `.pck`: header, then every bank and stream entry with
/// the media ids `extract_media` accepts. Reads files only.
///
/// Pure file-format work, works outside the game. Raises on a missing or
/// malformed archive.
#[gen_stub_pyfunction]
#[pyfunction]
pub fn pck_probe(path: String) -> PyResult<String> {
    audio::probe(&PathBuf::from(path)).map_err(audio_error)
}
