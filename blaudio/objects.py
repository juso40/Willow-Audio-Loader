"""Create the Unreal objects that make new audio referenceable in-game.

Raw Wwise ids only reach `play_id`. Everything else in the game (Kismet
`Behavior_PostAkEvent`, `AnimNotify_AkEvent`, other mods listing
`find_all("AkEvent")`, ...) works with `AkEvent`/`AkBank` *objects*, so
`add` creates one bank object and one event object per sound.

Id wiring comes from the game's own script (`Engine/Classes/AkObject.uc`
and `AkEvent.uc`): `AkObject.ShortId` is the Wwise id on every Ak
object, and `AkEvent.RequiredBank` links an event to its `AkBank`.

Objects go under the `Transient` package with keep-alive object flags.
"""



import unrealsdk
from mods_base import ENGINE
from unrealsdk import unreal
from unrealsdk.unreal import UObject

__all__ = [
    "DEFAULT_PACKAGE",
    "KEEP_ALIVE_FLAGS",
    "create_bank",
    "create_event",
    "find_event",
    "object_path",
    "relink_event",
]

#: Conventional package name for callers that want `Transient.blaudio`.
#: Bare names are placed directly under `Transient`; dotted names create
#: package Outers. Named additions use their namespace instead.
DEFAULT_PACKAGE = "blaudio"

#: Object flags that keep a constructed object alive across garbage
#: collections. `0x4000` is the keep-alive bit unrealsdk itself uses
#: (`keep_alive_willow` in its sources), `0x4_0000_0000` the companion
#: bit that persistent object construction uses (the usual
#: `RF_Public|RF_Standalone|RF_ROOT_SET` recipe).
KEEP_ALIVE_FLAGS = 0x400004000


def object_path(obj: UObject) -> str:
    """Full object path (e.g. `Transient.blaudio.my_sound`), for logging."""
    return ENGINE.PathName(obj)


def create_bank(name: str, bank_id: int) -> UObject:
    """Create (or rebind) an `AkBank` object for a bank id.

    `name` is `Object` or `Package.Object`, relative to `Transient`.
    When the object already exists the call reuses it and sets its
    `ShortId` to `bank_id`, so re-running `add` on mod reload rebinds cleanly.
    """
    obj = _construct("AkBank", name)
    with unreal.notify_changes():
        obj.ShortId = _signed_short_id(bank_id)
    return obj


def create_event(name: str, event_id: int, bank: UObject | None = None) -> UObject:
    """Create (or rebind) an `AkEvent` object for an event id.

    Sets `ShortId` (the Wwise id `PlayAkEvent` posts) and, when `bank` is
    given, the `RequiredBank` link to an `AkBank` object from
    `create_bank`.
    """
    # A qualified event below its bank must use that AkBank as its Outer,
    # not create a Package with the same path as the bank.
    bank_path = object_path(bank) if bank is not None else ""
    qualified = name if name.startswith("Transient.") else f"Transient.{name}"
    if bank is not None and qualified.casefold().startswith(f"{bank_path.casefold()}."):
        new_name = qualified[len(bank_path) + 1 :]  # remove the outer and the dot from our name
        obj = _construct("AkEvent", new_name, outer=bank)
    else:
        obj = _construct("AkEvent", name)
    with unreal.notify_changes():
        obj.ShortId = _signed_short_id(event_id)
        if bank is not None:
            obj.RequiredBank = bank
    return obj


def find_event(target: str | unreal.UObject) -> UObject:
    """Resolve an existing loaded AkEvent by object or path. Never create one."""
    event = unrealsdk.find_object("AkEvent", target) if isinstance(target, str) else target
    if event not in unrealsdk.find_all("AkEvent", exact=False):
        raise ValueError("replacement target must be an existing loaded AkEvent or its path")
    return event


def relink_event(event: UObject, event_id: int, bank: UObject) -> None:
    """Point an existing event at a loaded replacement bank for this session.

    Keep the object identity so every existing reference sees the new audio.
    Restore both properties if either assignment fails. Never save packages.
    """
    previous_id, previous_bank = event.ShortId, event.RequiredBank
    with unreal.notify_changes():
        try:
            event.ShortId = _signed_short_id(event_id)
            event.RequiredBank = bank
        except Exception:
            event.ShortId = previous_id
            event.RequiredBank = previous_bank
            raise


def _signed_short_id(value: int) -> int:
    """Unreal stores Wwise's unsigned id in a signed 32-bit property."""
    value &= 0xFFFFFFFF
    return value if value < 0x80000000 else value - 0x100000000


def _construct(cls: str, name: str, *, outer: UObject | None = None) -> UObject:
    """Find-or-create an object, constructing real package Outers for dotted names."""
    if outer is None:
        outer = unrealsdk.find_object("Package", "Transient")
        name = name.removeprefix("Transient.")
    components = name.split(".")
    for component in components[:-1]:
        outer = _construct("Package", component, outer=outer)
    name = components[-1]
    path = f"{ENGINE.PathName(outer)}.{name}"
    try:
        return unrealsdk.find_object(cls, path)
    except ValueError:
        pass
    template = unrealsdk.find_class(cls).ClassDefaultObject
    gobj = unrealsdk.construct_object(
        cls=cls,
        outer=outer,
        name=name,
        template_obj=template,
    )
    gobj.ObjectFlags |= KEEP_ALIVE_FLAGS
    return gobj
