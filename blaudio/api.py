"""Low-level wrappers over Rust: explicit ids, audio bytes, and engine calls.

This layer neither allocates ids nor creates Unreal objects. Use
`blaudio.runtime` (also exported at package level) for those operations.
"""

import sys
from collections.abc import Mapping
from pathlib import Path
from typing import Literal, get_args

from . import _blaudio

__all__ = [
    "POSITIONING_MODES",
    "Positioning",
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
    "to_wem",
    "unload_bank",
    "unregister_game_object",
]

AudioSource = str | Path | bytes
SoundMap = Mapping[int, AudioSource]
Positioning = Literal["2d", "2d_panned", "3d", "3d_static", "3d_no_attenuation", "3d_unspatialized", "weapon"]
POSITIONING_MODES: tuple[str, ...] = get_args(Positioning)
PositioningMap = Positioning | dict[int, Positioning]


def numeric_id(value: int, label: str) -> int:
    """Validate an explicit, nonzero unsigned Wwise id."""
    if isinstance(value, bool) or not isinstance(value, int) or not 0 < value <= 0xFFFFFFFF:
        raise ValueError(f"{label} must be an integer between 1 and 0xFFFFFFFF")
    return value


def positioning_modes[Key](
    sounds: Mapping[Key, AudioSource], positioning: Positioning | dict[Key, Positioning]
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


def craft_bank(
    bank_id: int,
    sounds: SoundMap,
    bank_name: str | None = None,
    *,
    positioning: PositioningMap = "3d",
) -> bytes:
    """Build a bank using explicit bank/event ids. Do not load or reserve it.

    `sounds` maps numeric event ids to audio paths or bytes. WAV, MP3,
    FLAC, OGG, AAC and `.wem` input are supported. Non-`.wem` input is
    decoded to PCM and embedded as a PCM `.wem`, preserving source
    channels and sample rate. `bank_name` names the `STID` entry (default
    `BlAudio_<bank_id>`).

    `positioning` accepts a global mode or per-event-id overrides;
    omitted overrides default to 3D. See README.md for the modes.

    Pass the result to `load_bank`, or use `blaudio.add` for automatic
    allocation, collision checks, loading, and Unreal object creation.
    """
    numeric_id(bank_id, "bank_id")
    modes = positioning_modes(sounds, positioning)
    event_ids = [numeric_id(event_id, "event id") for event_id in sounds]
    audio = [source if isinstance(source, bytes) else Path(source).read_bytes() for source in sounds.values()]
    return bytes(_blaudio.craft_bank(bank_id, bank_name, event_ids, audio, list(modes.values())))


def load_bank(data: bytes | str | Path) -> int:
    """Load a bank into Wwise from memory and return its bank id.

    Accept bytes from `craft_bank` or a path to a `.bnk`. Calls
    `AK::SoundEngine::LoadBank`; the bank's events are playable as soon
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

    Includes bank-embedded media through `DIDX`. Reads archives only;
    the ids can be passed to `extract_media`.
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
