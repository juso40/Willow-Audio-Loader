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
//! and run outside the game too. Engine calls fail without game symbols.
//! Unreal object creation and high-level playback live in the Python
//! package (`blaudio.objects` and `blaudio.runtime`).
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

/// Stop an event's instances, optionally restricted to a game object.
///
/// None targets all emitters, not game object 0. `fade_ms` uses a linear
/// fade. True means Wwise accepted the queued action, not that playback
/// has already stopped. False when unresolved, unknown, or off-game.
#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (event_id, game_object = None, fade_ms = 0))]
pub fn stop_event(event_id: u32, game_object: Option<u32>, fade_ms: i32) -> PyResult<bool> {
    if fade_ms < 0 {
        return Err(PyValueError::new_err("fade_ms must be nonnegative"));
    }
    Ok(audio::wwise::stop_event(event_id, game_object, fade_ms))
}

/// Stop one playing id returned by post_event, with an optional linear fade.
///
/// True means the native void API was called, not a playback-status check.
/// False for playing id 0, when unresolved, or off-game.
#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (playing_id, fade_ms = 0))]
pub fn stop_playing_id(playing_id: u32, fade_ms: i32) -> PyResult<bool> {
    if fade_ms < 0 {
        return Err(PyValueError::new_err("fade_ms must be nonnegative"));
    }
    Ok(audio::wwise::stop_playing_id(playing_id, fade_ms))
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
/// `3d_no_attenuation`, `3d_unspatialized`, or `perspective`.
/// Default `3d` preserves the donor behavior. `perspective` selects first-person 2D and
/// third-person 3D using the game's switch. `looping` optionally supplies
/// one boolean per event (default false). True repeats until stopped.
/// `stop_event_ids` optionally supplies a companion Stop event id per
/// sound (None entries omit it), scoped to the posting game object.
#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (bank_id, bank_name, event_ids, wems, positioning = None, looping = None, stop_event_ids = None))]
pub fn craft_bank(
    bank_id: u32,
    bank_name: Option<String>,
    event_ids: Vec<u32>,
    wems: Vec<Vec<u8>>,
    positioning: Option<Vec<String>>,
    looping: Option<Vec<bool>>,
    stop_event_ids: Option<Vec<Option<u32>>>,
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
    let loops = looping.unwrap_or_else(|| vec![false; event_ids.len()]);
    if loops.len() != event_ids.len() {
        return Err(PyValueError::new_err(
            "looping and event_ids must have the same length",
        ));
    }
    let stops = stop_event_ids.unwrap_or_else(|| vec![None; event_ids.len()]);
    if stops.len() != event_ids.len() {
        return Err(PyValueError::new_err(
            "stop_event_ids and event_ids must have the same length",
        ));
    }
    let sounds: Vec<audio::craft::NewSound> = event_ids
        .into_iter()
        .zip(wems)
        .zip(modes)
        .zip(loops)
        .zip(stops)
        .map(
            |((((event_id, media), positioning), looping), stop_event_id)| audio::craft::NewSound {
                event_id,
                media,
                size_hint: None,
                positioning,
                looping,
                stop_event_id,
            },
        )
        .collect();
    let name = bank_name.unwrap_or_else(|| format!("BlAudio_{bank_id:08X}"));
    audio::craft::craft_bank(bank_id, &name, &sounds).map_err(audio_error)
}

/// Build a bank of validated selection-tree specs (normally use Python SoundPool/Switch).
/// Pools are step containers: one selection per event post. State/switch
/// branches may nest. None children are explicitly silent. Routing chooses
/// an existing Init-bank bus, never creates or changes a global game state.
/// `gain` optionally supplies one dB offset per event (-96..+12, default 0).
#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (bank_id, bank_name, event_ids, sources, positioning, looping, stop_event_ids, routing, gain = None))]
#[allow(clippy::too_many_arguments)]
pub fn craft_tree_bank(
    bank_id: u32,
    bank_name: Option<String>,
    event_ids: Vec<u32>,
    sources: Vec<Bound<'_, PyAny>>,
    positioning: Vec<String>,
    looping: Vec<bool>,
    stop_event_ids: Vec<Option<u32>>,
    routing: Vec<String>,
    gain: Option<Vec<f32>>,
) -> PyResult<Vec<u8>> {
    let n = event_ids.len();
    let gains = gain.unwrap_or_else(|| vec![0.0; n]);
    if [
        sources.len(),
        positioning.len(),
        looping.len(),
        stop_event_ids.len(),
        routing.len(),
        gains.len(),
    ]
    .iter()
    .any(|length| *length != n)
    {
        return Err(PyValueError::new_err(
            "all event option arrays must have the same length",
        ));
    }
    let mut events = Vec::with_capacity(n);
    for (i, event_id) in event_ids.into_iter().enumerate() {
        let routing = match routing[i].as_str() {
            "sfx" => audio::craft::Routing::Sfx,
            "voice" => audio::craft::Routing::Voice,
            "player_voice" => audio::craft::Routing::PlayerVoice,
            _ => return Err(PyValueError::new_err("unknown routing mode")),
        };
        let mut nodes = 0;
        events.push(audio::craft::NewEvent {
            event_id,
            source: parse_source(&sources[i], 0, &mut nodes)?,
            positioning: positioning[i]
                .parse()
                .map_err(|e: audio::AudioError| PyValueError::new_err(e.to_string()))?,
            looping: looping[i],
            stop_event_id: stop_event_ids[i],
            routing,
            gain: gains[i],
        });
    }
    let name = bank_name.unwrap_or_else(|| format!("BlAudio_{bank_id:08X}"));
    audio::craft::craft_events(bank_id, &name, &events).map_err(audio_error)
}

fn parse_source(
    value: &Bound<'_, PyAny>,
    depth: usize,
    nodes: &mut usize,
) -> PyResult<audio::craft::SoundSource> {
    use audio::craft::{Selection, SoundSource};
    *nodes += 1;
    if depth > 16 || *nodes > 65535 {
        return Err(PyValueError::new_err("sound tree exceeds depth/node limit"));
    }
    let kind: String = value.get_item("type")?.extract()?;
    match kind.as_str() {
        "clip" => Ok(SoundSource::Clip {
            media: value.get_item("audio")?.extract()?,
            size_hint: None,
        }),
        "pool" => {
            let selection: String = value.get_item("selection")?.extract()?;
            let selection = match selection.as_str() {
                "random" => Selection::Random,
                "shuffle" => Selection::Shuffle,
                "sequence" => Selection::Sequence,
                _ => return Err(PyValueError::new_err("unknown selection mode")),
            };
            let clips: Vec<Bound<'_, PyAny>> = value.get_item("clips")?.extract()?;
            *nodes += clips.len();
            if depth == 16 || *nodes > 65535 {
                return Err(PyValueError::new_err("sound tree exceeds depth/node limit"));
            }
            let clips = clips
                .iter()
                .map(|clip| {
                    if clip.get_item("type")?.extract::<String>()? != "clip" {
                        return Err(PyValueError::new_err("pool entries must be clips"));
                    }
                    clip.get_item("audio")?.extract()
                })
                .collect::<PyResult<Vec<Vec<u8>>>>()?;
            Ok(SoundSource::Pool {
                clips,
                selection,
                weights: value.get_item("weights")?.extract()?,
                avoid_repeat: value.get_item("avoid_repeat")?.extract()?,
                global: value.get_item("global")?.extract()?,
            })
        }
        "switch" => {
            let kind: String = value.get_item("kind")?.extract()?;
            if kind != "switch" && kind != "state" {
                return Err(PyValueError::new_err("unknown switch kind"));
            }
            let entries: Vec<(u32, Bound<'_, PyAny>)> = value.get_item("branches")?.extract()?;
            if entries.len() > 65535 {
                return Err(PyValueError::new_err("too many branches"));
            }
            let branches = entries
                .into_iter()
                .map(|(id, child)| {
                    let source = if child.is_none() {
                        None
                    } else {
                        Some(parse_source(&child, depth + 1, nodes)?)
                    };
                    Ok((id, source))
                })
                .collect::<PyResult<Vec<_>>>()?;
            Ok(SoundSource::Switch {
                group: value.get_item("group")?.extract()?,
                is_state: kind == "state",
                default: value.get_item("default")?.extract()?,
                branches,
            })
        }
        _ => Err(PyValueError::new_err("unknown source type")),
    }
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
