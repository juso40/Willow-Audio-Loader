import json
from collections.abc import Mapping
from hashlib import sha256
from typing import Any, NamedTuple

import unrealsdk
from mods_base import get_pc
from unrealsdk import unreal
from unrealsdk.unreal import UObject

from . import api
from .api import Positioning
from .ids import SoundKey, identifier, registry, stop_event_names
from .objects import create_bank, create_event, find_event, object_path, relink_event
from .sources import Routing, SoundSource, extra_masks, read_source, resolved_source, source_spec

__all__ = ["AddResult", "add", "get_added_event", "play", "play_id", "replace", "replace_bytes", "stop"]

ObjectLike = str | UObject
SoundMap = Mapping[SoundKey, SoundSource]
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


def add(  # noqa: PLR0913 - keep explicit keyword-only options for callers
    sounds: SoundMap,
    *,
    namespace: str,
    bank_name: str,
    positioning: PositioningMap = "3d",
    looping: bool | dict[SoundKey, bool] = False,
    stop_events: Mapping[SoundKey, str] | None = None,
    routing: Routing | dict[SoundKey, Routing] = "sfx",
    gain: float | dict[SoundKey, float] = 0,
) -> AddResult:
    """Allocate ids, load a sound bank, and create its AkBank/AkEvent objects.

    Like a manifest, every call requires namespace, bank_name, and named
    sounds. The same identifier restrictions and case-insensitive name
    uniqueness apply. All ids are allocated internally for this session.
    Banks use `Transient.<namespace>.<bank_name>` and events below that
    bank as `<name>`. For explicit ids, use api.craft_bank and api.load_bank.

    Sources may be audio paths/bytes, SoundPool, or nested Switch trees.
    `routing` accepts sfx, voice, or player_voice (global or per event).
    `gain` is a global/per-event offset in dB (-96..+12, default 0).
    `positioning` accepts a global mode or per-sound overrides (default 3D).
    `looping` accepts a global boolean or per-sound overrides (default false).
    True repeats until stopped. Posting again starts another instance.
    `stop_events` maps sound names to companion event names. These are
    returned in `events` too and stop the sound on the actor posting them.
    Companion names must be unique among all event names in the bank.
    The registry checks loaded AkObjects and our reserved ids, including
    derived HIRC ids. It cannot see unloaded game banks or arbitrary
    external Wwise nodes. Repeating a bank with different bytes requires
    a restart. Allocations last for this session, not across restarts.

    ```python
    result = add(
        {"shot": "shot.wav", "reload": "reload.mp3"},
        namespace="my_mod", bank_name="Weapons", positioning="perspective",
    )
    play(result.events["shot"])
    ```
    """
    # Validate identifiers before reserving ids or constructing objects.
    identifier(namespace, "namespace")
    identifier(bank_name, "bank_name")
    for name in sounds:
        identifier(name, "sound name")

    modes: dict[SoundKey, Positioning] = api.positioning_modes(sounds, positioning)
    loops = api.looping_modes(sounds, looping)
    routes = api.routing_modes(sounds, routing)
    gains = api.gain_modes(sounds, gain)
    specs = {name: source_spec(source) for name, source in sounds.items()}
    masks = {name: extra_masks(spec, modes[name]) for name, spec in specs.items()}
    stops = stop_event_names(sounds, stop_events)
    object_name = f"{namespace}.{bank_name}"
    with registry.reserve(bank_name, namespace, modes, stop_events=stops, source_masks=masks) as allocation:
        # Use the validated shape, not mutable caller-owned lists/mappings.
        # Unreal lookups or other callbacks must not change the tree after
        # its derived ids have been reserved.
        resolved = {allocation.event_ids[key]: resolved_source(spec) for key, spec in specs.items()}
        resolved_modes: dict[int, Positioning] = {allocation.event_ids[key]: mode for key, mode in modes.items()}
        resolved_loops = {allocation.event_ids[key]: loop for key, loop in loops.items()}
        resolved_stops = {allocation.event_ids[key]: allocation.event_ids[name] for key, name in stops.items()}

        _check_named_object("AkBank", object_name, allocation.bank_id)
        for name, event_id in allocation.event_ids.items():
            _check_named_object("AkEvent", f"{object_name}.{name}", event_id)

        data = api.craft_bank(
            allocation.bank_id,
            resolved,
            bank_name=object_name,
            positioning=resolved_modes,
            looping=resolved_loops,
            stop_events=resolved_stops,
            routing={allocation.event_ids[key]: route for key, route in routes.items()},
            gain={allocation.event_ids[key]: value for key, value in gains.items()},
        )
        registry.check_bank(allocation, data)
        loaded_id = api.load_bank(data)
        registry.commit(allocation, data)
        bank_obj = create_bank(object_name, loaded_id)
        registry.track_objects([bank_obj])

        events: dict[SoundKey, Any] = {}
        for name, event_id in allocation.event_ids.items():
            event = create_event(f"{object_name}.{name}", event_id, bank_obj)
            if routes.get(name, "sfx") != "sfx":
                # Only Play events are voices. Companion Stops carry no speech.
                with unreal.notify_changes():
                    event.bVoice = True
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
    or load anything. name is a sound or companion Stop event name,
    not its object path.
    """
    return registry.get_event(name, namespace, bank_name)


def replace(
    sound: SoundSource,
    target: ObjectLike,
    *,
    positioning: Positioning = "3d",
    routing: Routing = "sfx",
    gain: float = 0,
) -> str:
    """Relink an existing AkEvent to an audio clip or selection tree.

    All callers of that Unreal object see the change. Raw posts of its old
    id do not. This replaces, rather than merges, the original Wwise graph.
    Identical target/content/settings reuse a bank. Changed content loads a
    new bank, retained until game exit. Never modify game files.
    """
    event = find_event(target)
    path = object_path(event)
    positioning = api.positioning_modes({"sound": sound}, positioning)["sound"]
    api.routing_modes({"sound": sound}, routing)
    gain = api.gain_value(gain)
    spec = read_source(source_spec(sound))
    # Hash a framed tree, not concatenated media. Branch ids, order, weights,
    # selection mode, scope, routing and every clip contribute to the key.
    description = json.dumps(
        spec,
        sort_keys=True,
        default=lambda data: {
            "sha256": sha256(data).hexdigest(),
            "length": len(data),
        },
    )
    digest = sha256(json.dumps([path, positioning, routing, description, gain]).encode()).hexdigest()
    name = f"Replacement_{digest}"
    modes: dict[SoundKey, Positioning] = {"sound": positioning}
    with registry.reserve(
        name, "blaudio_replace", modes, source_masks={"sound": extra_masks(spec, positioning)}
    ) as allocation:
        event_id = allocation.event_ids["sound"]
        object_name = f"blaudio_replace_{name}_{allocation.bank_id:08X}"
        bank_data = api.craft_bank(
            allocation.bank_id,
            {event_id: resolved_source(spec)},
            bank_name=object_name,
            positioning=positioning,
            routing=routing,
            gain=gain,
        )
        registry.check_bank(allocation, bank_data)
        loaded_id = api.load_bank(bank_data)
        registry.commit(allocation, bank_data)
        bank = create_bank(object_name, loaded_id)
        registry.track_objects([bank])
        relink_event(event, event_id, bank, voice=True if routing != "sfx" else None)
        registry.track_objects([event])
    return f"relink {path} to event 0x{event_id:08x} in runtime bank 0x{loaded_id:08x} ({positioning}, {routing})"


def replace_bytes(
    data: bytes, target: ObjectLike, *, positioning: Positioning = "3d", routing: Routing = "sfx", gain: float = 0
) -> str:
    """Like replace, for in-memory audio bytes. Never create an AkEvent."""
    return replace(data, target, positioning=positioning, routing=routing, gain=gain)


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


def stop(event: ObjectLike, *, fade_ms: int = 0) -> bool:
    """Stop an AkEvent's instances on all emitters, optionally fading out.

    This includes instances posted by other callers using the same event.
    Use api.stop_event for emitter-scoped stopping or api.stop_playing_id
    for one raw-post instance. True means Wwise accepted the queued action,
    not that playback has already stopped. Does not create or unload banks.
    """
    return api.stop_event(int(find_event(event).ShortId), fade_ms=fade_ms)


def play_id(event_id: int) -> None:
    """Play a loaded Wwise event id through a lightweight AkEvent object."""
    play(create_event(f"Ev{event_id & 0xFFFFFFFF:08X}", event_id & 0xFFFFFFFF))
