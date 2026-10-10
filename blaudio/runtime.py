"""High-level audio operations: reserve ids, load banks, and wire game objects.

Reservations are released on pre-load failure and committed immediately
once Wwise holds the bank. Unreal object creation must not undo that commit.
"""


from collections.abc import Mapping
from hashlib import sha256
from pathlib import Path
from typing import Any, NamedTuple

import unrealsdk
from mods_base import get_pc
from unrealsdk.unreal import UObject

from . import api
from .api import POSITIONING_MODES, Positioning
from .ids import SoundKey, identifier, registry
from .objects import create_bank, create_event, find_event, object_path, relink_event

__all__ = ["AddResult", "add", "get_added_event", "play", "play_id", "replace", "replace_bytes"]

ObjectLike = str | UObject
SoundMap = Mapping[SoundKey, api.AudioSource]
PositioningMap = Positioning | dict[SoundKey, Positioning]


class AddResult(NamedTuple):
    """A loaded bank and its Unreal objects, keyed by sound name."""

    bank_id: int
    bank: Any
    events: dict[SoundKey, Any]

    @property
    def event_ids(self) -> dict[SoundKey, int]:
        """Unsigned Wwise ids for the returned events, valid this session."""
        return {key: int(event.ShortId) & 0xFFFFFFFF for key, event in self.events.items()}


def add(
    sounds: SoundMap,
    *,
    namespace: str,
    bank_name: str,
    positioning: PositioningMap = "3d",
) -> AddResult:
    """Allocate ids, load a sound bank, and create its AkBank/AkEvent objects.

    Like a manifest, every call requires namespace, bank_name, and named
    sounds. The same identifier restrictions and case-insensitive name
    uniqueness apply. All ids are allocated internally for this session.
    Banks use `Transient.<namespace>.<bank_name>` and events below that
    bank as `<name>`. For explicit ids, use api.craft_bank and api.load_bank.

    `positioning` accepts a global mode or per-sound overrides (default 3D).
    The registry checks loaded AkObjects and our reserved ids, including
    derived HIRC ids. It cannot see unloaded game banks or arbitrary
    external Wwise nodes. Repeating a bank with different bytes requires
    a restart. Allocations last for this session, not across restarts.

    ```python
    result = add(
        {"shot": "shot.wav", "reload": "reload.mp3"},
        namespace="my_mod", bank_name="Weapons", positioning="weapon",
    )
    play(result.events["shot"])
    ```
    """
    # validate the identifiers
    identifier(namespace, "namespace")
    identifier(bank_name, "bank_name")
    for name in sounds:
        identifier(name, "sound name")

    modes: dict[SoundKey, Positioning] = api.positioning_modes(sounds, positioning)
    object_name = f"{namespace}.{bank_name}"
    with registry.reserve(bank_name, namespace, modes) as allocation:
        resolved = {allocation.event_ids[key]: source for key, source in sounds.items()}
        resolved_modes: dict[int, Positioning] = {allocation.event_ids[key]: mode for key, mode in modes.items()}

        _check_named_object("AkBank", object_name, allocation.bank_id)
        for name, event_id in allocation.event_ids.items():
            _check_named_object("AkEvent", f"{object_name}.{name}", event_id)

        data = api.craft_bank(allocation.bank_id, resolved, bank_name=object_name, positioning=resolved_modes)
        registry.check_bank(allocation, data)
        loaded_id = api.load_bank(data)
        registry.commit(allocation, data)
        bank_obj = create_bank(object_name, loaded_id)
        registry.track_objects([bank_obj])

        events: dict[SoundKey, Any] = {}
        for name, event_id in allocation.event_ids.items():
            event = create_event(f"{object_name}.{name}", event_id, bank_obj)
            registry.track_objects([event])
            registry.remember_event(event)
            events[name] = event

        return AddResult(loaded_id, bank_obj, events)


def _check_named_object(cls: str, name: str, expected_id: int) -> None:
    """Do not silently rebind somebody else's object named path."""
    try:
        obj = unrealsdk.find_object(cls, f"Transient.{name}")
    except ValueError:
        return
    if int(obj.ShortId) & 0xFFFFFFFF != expected_id:
        raise ValueError(f"{cls}'Transient.{name}' already exists with a different audio id")


def get_added_event(name: str, *, namespace: str, bank_name: str) -> Any:
    """Get a previously added named event, comparing names case-insensitively.

    Raises KeyError if no event was created this session. Does not allocate
    or load anything. name is the original sound key, not its object path.
    """
    return registry.get_event(name, namespace, bank_name)


def replace(sound: str | Path, target: ObjectLike, *, positioning: Positioning = "3d") -> str:
    """Relink an existing AkEvent to new audio, without modifying game files.

    Keep the object identity so existing references play the new audio.
    Raw Wwise posts of the original id keep their original behavior.
    Cannot infer the original positioning or preserve a multi-action graph.

    Repeating a target, audio and mode reuses its replacement bank.
    Changed audio or mode loads a new bank. Keep all allocated ids and
    loaded buffers until the game exits, including earlier replacements.
    """
    return replace_bytes(Path(sound).read_bytes(), target, positioning=positioning)


def replace_bytes(data: bytes, target: ObjectLike, *, positioning: Positioning = "3d") -> str:
    """Like replace, for in-memory audio bytes. Never create an AkEvent."""
    event = find_event(target)
    path = object_path(event)
    if not isinstance(positioning, str) or positioning not in POSITIONING_MODES:
        raise ValueError(f"invalid positioning {positioning!r}. Expected {', '.join(POSITIONING_MODES)}")

    modes: dict[SoundKey, Positioning] = {"sound": positioning}
    digest = sha256(path.encode() + b"\0" + positioning.encode() + b"\0" + data).hexdigest()
    name = f"Replacement_{digest}"

    with registry.reserve(name, "blaudio_replace", modes) as allocation:
        event_id = allocation.event_ids["sound"]
        object_name = f"blaudio_replace_{name}_{allocation.bank_id:08X}"

        bank_data = api.craft_bank(allocation.bank_id, {event_id: data}, bank_name=object_name, positioning=positioning)
        registry.check_bank(allocation, bank_data)

        loaded_id = api.load_bank(bank_data)
        registry.commit(allocation, bank_data)

        bank = create_bank(object_name, loaded_id)
        registry.track_objects([bank])

        relink_event(event, event_id, bank)
        registry.track_objects([event])

    return f"relink {path} to event 0x{event_id:08x} in runtime bank 0x{loaded_id:08x} ({positioning})"


def play(event: ObjectLike) -> None:
    """Play by AkEvent object or path, posting from the player's pawn.

    The controller has no world location, so posting spatialized audio
    from it would leave the sound at the world origin.
    """
    if isinstance(event, str):
        event = unrealsdk.find_object("AkEvent", event)
    pc = get_pc()
    actor = pc.Pawn if pc.Pawn is not None else pc
    actor.PlayAkEvent(event)


def play_id(event_id: int) -> None:
    """Play a loaded Wwise event id through a lightweight AkEvent object."""
    play(create_event(f"Ev{event_id & 0xFFFFFFFF:08X}", event_id & 0xFFFFFFFF))
