import sys
from collections.abc import Mapping
from pathlib import Path
from typing import Literal, get_args

from . import _blaudio
from .sources import ROUTING_MODES, AudioSource, Routing, SoundPool, SoundSource, Switch, read_source, source_spec

__all__ = [
    "POSITIONING_MODES",
    "AudioSource",
    "Positioning",
    "Routing",
    "SoundPool",
    "Switch",
    "craft_bank",
    "extract_media",
    "get_id_from_string",
    "load_bank",
    "media_list",
    "post_event",
    "probe",
    "register_game_object",
    "resolve_pck",
    "set_active_listeners",
    "stop_event",
    "stop_playing_id",
    "to_wem",
    "unload_bank",
    "unregister_game_object",
]

SoundMap = Mapping[int, SoundSource]
Positioning = Literal["2d", "2d_panned", "3d", "3d_static", "3d_no_attenuation", "3d_unspatialized", "perspective"]
POSITIONING_MODES: tuple[str, ...] = get_args(Positioning)
PositioningMap = Positioning | dict[int, Positioning]


def numeric_id(value: int, label: str) -> int:
    """Validate an explicit, nonzero unsigned Wwise id."""
    if isinstance(value, bool) or not isinstance(value, int) or not 0 < value <= 0xFFFFFFFF:
        raise ValueError(f"{label} must be an integer between 1 and 0xFFFFFFFF")
    return value


def positioning_modes[Key](
    sounds: Mapping[Key, object], positioning: Positioning | dict[Key, Positioning]
) -> dict[Key, Positioning]:
    """Normalize a global mode or per-sound overrides, defaulting to 3D."""
    modes: dict[Key, Positioning]
    if isinstance(positioning, dict):
        unknown = positioning.keys() - sounds.keys()
        if unknown:
            raise ValueError(f"positioning contains unknown sound keys: {sorted(map(repr, unknown))}")
        modes = {key: positioning.get(key, "3d") for key in sounds}
    else:
        modes = dict.fromkeys(sounds, positioning)
    for key, mode in modes.items():
        if not isinstance(mode, str) or mode not in POSITIONING_MODES:
            raise ValueError(f"invalid positioning {mode!r} for {key!r}. Expected {', '.join(POSITIONING_MODES)}")
    return modes


def looping_modes[Key](sounds: Mapping[Key, object], looping: bool | dict[Key, bool]) -> dict[Key, bool]:
    """Normalize global/per-sound looping, defaulting omitted overrides to false."""
    if isinstance(looping, dict):
        if unknown := looping.keys() - sounds.keys():
            raise ValueError(f"looping contains unknown sound keys: {sorted(map(repr, unknown))}")
        modes = {key: looping.get(key, False) for key in sounds}
    else:
        if not isinstance(looping, bool):
            raise ValueError(f"looping must be a boolean or a dictionary, got {looping!r}")
        modes = dict.fromkeys(sounds, looping)
    for key, mode in modes.items():
        if not isinstance(mode, bool):
            raise ValueError(f"looping must be a boolean for {key!r}, got {mode!r}")
    return modes


def routing_modes[Key](sounds: Mapping[Key, object], routing: Routing | dict[Key, Routing]) -> dict[Key, Routing]:
    """Normalize existing Init-bus routes without changing global state."""
    if isinstance(routing, dict):
        if unknown := routing.keys() - sounds.keys():
            raise ValueError(f"routing contains unknown sound keys: {sorted(map(repr, unknown))}")
        modes = {key: routing.get(key, "sfx") for key in sounds}
    else:
        modes = dict.fromkeys(sounds, routing)
    for key, mode in modes.items():
        if not isinstance(mode, str) or mode not in ROUTING_MODES:
            raise ValueError(f"invalid routing {mode!r} for {key!r}. Expected {', '.join(ROUTING_MODES)}")
    return modes


def gain_value(value: float, label: str = "gain") -> float:
    """Validate a decibel offset, not a linear amplitude multiplier."""
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not -96 <= value <= 12:
        raise ValueError(f"{label} must be a finite number between -96 and +12 dB")
    return float(value) if value else 0.0


def gain_modes[Key](sounds: Mapping[Key, object], gain: float | dict[Key, float]) -> dict[Key, float]:
    """Normalize global/per-event gain in dB. Missing overrides mean 0 dB."""
    if isinstance(gain, dict):
        if unknown := gain.keys() - sounds.keys():
            raise ValueError(f"gain contains unknown sound keys: {sorted(map(repr, unknown))}")
        return {key: gain_value(gain.get(key, 0), f"gain for {key!r}") for key in sounds}
    return dict.fromkeys(sounds, gain_value(gain))


def game_root() -> Path:
    """Resolve the game root from `<root>/Binaries/Win32/Borderlands2.exe`."""
    return Path(sys.executable).parent.parent.parent


def resolve_pck(value: str | Path) -> Path:
    """Resolve a `.pck` reference to a file path.

    Absolute paths and existing relative paths pass through. A bare name
    (like `Audio_Streaming.pck`) resolves against the game's
    `WillowGame/CookedPCConsole`.
    """
    path = Path(value)
    if path.is_absolute() or path.exists():
        return path
    return game_root() / "WillowGame" / "CookedPCConsole" / path


def craft_bank(  # noqa: PLR0913 - independent bank options are keyword-only
    bank_id: int,
    sounds: SoundMap,
    bank_name: str | None = None,
    *,
    positioning: PositioningMap = "3d",
    looping: bool | dict[int, bool] = False,
    stop_events: Mapping[int, int] | None = None,
    routing: Routing | dict[int, Routing] = "sfx",
    gain: float | dict[int, float] = 0,
) -> bytes:
    """Build a bank using explicit bank/event ids. Do not load or reserve it.

    `sounds` maps numeric event ids to audio paths or bytes. WAV, MP3,
    FLAC, OGG, AAC and `.wem` input are supported. Non-`.wem` input is
    decoded to PCM and embedded as a PCM `.wem`, preserving source
    channels and sample rate. `bank_name` names the `STID` entry (default
    `BlAudio_<bank_id>`).

    `positioning` accepts a global mode or per-event-id overrides.
    Omitted overrides default to 3D. See README.md for the modes.
    `looping` accepts a global boolean or per-event-id overrides (default
    false). True repeats until stopped. Supplied .wem loop points are kept.
    `stop_events` maps Play event ids to companion Stop event ids, targeting
    the same sound/tree on the game object posting the companion. No new media.
    Sources may also be SoundPool or nested Switch trees (see voice-selection.md).
    `routing` is sfx (default), voice, or player_voice. The latter honors the
    game's Player_CallOuts state. Pool looping repeats the chosen clip only.
    `gain` is a global/per-event dB offset (-96..+12, default 0), applied
    once to the event's mixer, including every clip/perspective branch.

    Pass the result to `load_bank`, or use `blaudio.add` for automatic
    allocation, collision checks, loading, and Unreal object creation.
    """
    numeric_id(bank_id, "bank_id")
    # Validate every tree and option before reading any path or reserving ids.
    if not sounds:
        raise ValueError("craft_bank: no sounds")
    specs = [source_spec(source) for source in sounds.values()]
    routes = routing_modes(sounds, routing)
    modes = positioning_modes(sounds, positioning)
    loops = looping_modes(sounds, looping)
    gains = gain_modes(sounds, gain)
    event_ids = [numeric_id(event_id, "event id") for event_id in sounds]
    stops = {} if stop_events is None else stop_events
    if not isinstance(stops, Mapping):
        raise ValueError("stop_events must be a mapping of play event ids to stop event ids")
    if unknown := stops.keys() - sounds.keys():
        raise ValueError(f"stop_events contains unknown event ids: {sorted(map(repr, unknown))}")
    for play_id, stop_id in stops.items():
        numeric_id(play_id, "play event id")
        numeric_id(stop_id, "stop event id")
    if (
        any(spec["type"] != "clip" for spec in specs)
        or any(route != "sfx" for route in routes.values())
        or any(gains.values())
    ):
        return bytes(
            _blaudio.craft_tree_bank(
                bank_id,
                bank_name,
                event_ids,
                [read_source(spec) for spec in specs],
                list(modes.values()),
                list(loops.values()),
                [stops.get(event_id) for event_id in event_ids],
                list(routes.values()),
                list(gains.values()),
            )
        )
    audio = [read_source(spec)["audio"] for spec in specs]
    return bytes(
        _blaudio.craft_bank(
            bank_id,
            bank_name,
            event_ids,
            audio,
            list(modes.values()),
            list(loops.values()),
            [stops.get(event_id) for event_id in event_ids],
        )
    )


def load_bank(data: bytes | str | Path) -> int:
    """Load a bank into Wwise from memory and return its bank id.

    Accept bytes from `craft_bank` or a path to a `.bnk`. Calls
    `AK::SoundEngine::LoadBank`. The bank's events are playable as soon
    as this returns. Release with `unload_bank`.
    """
    if not isinstance(data, bytes):
        data = Path(data).read_bytes()
    return _blaudio.load_bank(data)


def unload_bank(bank_id: int) -> bool:
    """Unload one of our banks. Return False for banks we do not own."""
    return _blaudio.unload_bank(bank_id)


def to_wem(data: bytes | str | Path) -> bytes:
    """Normalize audio to `.wem`, without downmixing or resampling.

    A `.wem` passes through untouched. Other formats are decoded to
    16-bit PCM at the source's rate and channels and wrapped in a PCM
    `.wem`. `craft_bank` already performs this conversion for its inputs.
    """
    if not isinstance(data, bytes):
        data = Path(data).read_bytes()
    return bytes(_blaudio.to_wem(data))


def media_list(pck: str | Path) -> list[tuple[int, int]]:
    """List archive media as `(media_id, size)` pairs, streams first.

    Includes bank-embedded media through `DIDX`. Reads archives only.
    The ids can be passed to `extract_media`.
    """
    return _blaudio.media_list(str(resolve_pck(pck)))


def extract_media(pck: str | Path, target: int) -> bytes:
    """Read a stream or bank-embedded `.wem` unchanged from an archive."""
    return bytes(_blaudio.extract_media(str(resolve_pck(pck)), target & 0xFFFFFFFF))


def probe(pck: str | Path) -> str:
    """Dump a `.pck`'s headers, banks and streams. Reads files only."""
    return _blaudio.pck_probe(str(resolve_pck(pck)))


def post_event(event_id: int, game_object: int = 0) -> int:
    """Post a raw Wwise event id, bypassing Unreal, and return its playing id.

    Return 0 when the bank is not loaded or the event is unknown.
    Register and listener-bind the game object first. Ids are masked to
    32 bits because Unreal exposes ShortId as a signed integer.
    """
    return _blaudio.post_event(event_id & 0xFFFFFFFF, game_object & 0xFFFFFFFF)


def _fade_duration(fade_ms: int) -> int:
    if isinstance(fade_ms, bool) or not isinstance(fade_ms, int) or not 0 <= fade_ms <= 0x7FFFFFFF:
        raise ValueError("fade_ms must be an integer between 0 and 0x7FFFFFFF")
    return fade_ms


def stop_event(event_id: int, game_object: int | None = None, *, fade_ms: int = 0) -> bool:
    """Queue Stop for an event's instances, with an optional linear fade.

    `None` targets all emitters. Numeric game object 0 targets only that
    object. True means the action was accepted, not that a voice was found
    or the fade completed. False for unknown events or unavailable engine.
    """
    return _blaudio.stop_event(
        event_id & 0xFFFFFFFF,
        None if game_object is None else game_object & 0xFFFFFFFF,
        _fade_duration(fade_ms),
    )


def stop_playing_id(playing_id: int, *, fade_ms: int = 0) -> bool:
    """Stop one instance returned by `post_event`, with an optional linear fade.

    True only means the native void API was called. It is not a playback
    status check. False for id 0 or when the engine API is unavailable.
    """
    return _blaudio.stop_playing_id(playing_id & 0xFFFFFFFF, _fade_duration(fade_ms))


def register_game_object(game_object: int) -> bool:
    """Register a game object id with the sound engine."""
    return _blaudio.register_game_object(game_object & 0xFFFFFFFF)


def unregister_game_object(game_object: int) -> bool:
    """Unregister a game object id."""
    return _blaudio.unregister_game_object(game_object & 0xFFFFFFFF)


def set_active_listeners(game_object: int, mask: int = 1) -> bool:
    """Bind listener bits to a game object (bit 0 = listener 0)."""
    return _blaudio.set_active_listeners(game_object & 0xFFFFFFFF, mask & 0xFFFFFFFF)


def get_id_from_string(name: str) -> int:
    """Return Wwise's own name hash, or 0 for an empty name."""
    return _blaudio.get_id_from_string(name)
