"""Session-only allocation of bank, event and derived HIRC ids.

Checks loaded AkObject ids and allocations made through this module.
It cannot see unloaded banks, internal game HIRC nodes, or arbitrary
banks another mod loads directly into Wwise.
"""


import re
import secrets
from collections.abc import Generator, Mapping
from contextlib import contextmanager
from dataclasses import dataclass, field
from hashlib import sha256

import unrealsdk
from unrealsdk.unreal import UObject

from .objects import object_path

SoundKey = str
_MASK = 0xFFFFFFFF
_ATTENUATION_ID = 0x03F8F132
# Keep in sync with native/crates/audio/src/craft.rs. Named allocations
# reserve both weapon-only nodes too, even when initially used as 3D.
_DERIVED_MASKS = (0x534F554E, 0x41435449, 0x4D495852, 0x53574954, 0x54485244)
_IDENTIFIER = re.compile(r"[A-Za-z_][A-Za-z0-9_]*\Z")


def identifier(value: str, label: str) -> str:
    """Validate a namespace or automatic object's name component."""
    if not isinstance(value, str) or not _IDENTIFIER.fullmatch(value):
        raise ValueError(f"{label} must contain letters, digits or underscores and not start with a digit")
    return value


def _scope(bank_name: str, namespace: str, modes: Mapping[SoundKey, str]) -> str:
    if not modes:
        raise ValueError("add: no sounds")
    identifier(namespace, "namespace")
    identifier(bank_name, "bank_name")
    return f"{namespace.casefold()}/{bank_name.casefold()}"


@dataclass
class Allocation:
    """One bank's reservations, committed only after Wwise accepts it."""

    bank_id: int
    event_ids: dict[SoundKey, int]
    owner: str
    committed: bool = False
    new_ids: list[int] = field(default_factory=list)
    new_names: list[str] = field(default_factory=list)


class IdRegistry:
    """Owns allocations until the game exits, including unloaded banks."""

    def __init__(self) -> None:
        self._used: dict[int, str] = {_ATTENUATION_ID: "shared blaudio attenuation"}
        self._names: dict[str, int] = {}
        self._objects: dict[int, set[str]] = {}
        self._banks: dict[str, str] = {}
        self._events: dict[int, UObject] = {}

    def _loaded_ids(self) -> dict[int, str]:
        occupied: dict[int, str] = {}
        for obj in unrealsdk.find_all("AkObject", exact=False):
            value = int(obj.ShortId) & _MASK
            if not value:
                continue
            path = object_path(obj)
            if path not in self._objects.get(value, set()):
                occupied[value] = path
        return occupied

    def _conflict(self, ids: set[int], owner: str, loaded: dict[int, str]) -> tuple[int, str] | None:
        for value in sorted(ids):
            if value == 0:
                return value, "reserved zero id"
            used = self._used.get(value)
            if used is not None and used != owner:
                return value, used
            if value in loaded:
                return value, f"loaded AkObject {loaded[value]}"
        return None

    def _reserve(
        self,
        owner: str,
        masks: tuple[int, ...],
        loaded: dict[int, str],
        allocation: Allocation,
    ) -> int:
        candidate = self._names.get(owner)
        automatic = candidate is None
        for _ in range(4096):
            if automatic:
                candidate = secrets.randbits(32)
            assert candidate is not None
            family = {candidate, *(candidate ^ mask for mask in masks)}
            conflict = self._conflict(family, owner, loaded)
            if conflict is not None:
                if automatic:
                    continue
                value, source = conflict
                raise ValueError(f"audio id collision: 0x{value:08X} for {owner} conflicts with {source}")
            for value in family:
                if value not in self._used:
                    self._used[value] = owner
                    allocation.new_ids.append(value)
            if owner not in self._names:
                self._names[owner] = candidate
                allocation.new_names.append(owner)
            return candidate
        raise RuntimeError(f"could not allocate collision-free audio ids for {owner}")

    @contextmanager
    def reserve(
        self,
        bank_name: str,
        namespace: str,
        modes: Mapping[SoundKey, str],
    ) -> Generator[Allocation]:
        """Reserve all ids atomically. Release new ones on a pre-load failure."""
        scope = _scope(bank_name, namespace, modes)
        allocation = Allocation(0, {}, f"bank:{scope}")
        loaded = self._loaded_ids()
        try:
            allocation.bank_id = self._reserve(allocation.owner, (), loaded, allocation)
            seen: set[str] = set()
            for key in modes:
                identifier(key, "sound name")
                owner = f"event:{scope}/name:{key.casefold()}"
                if owner in seen:
                    raise ValueError(f"duplicate sound name ignoring case: {key!r}")
                seen.add(owner)
                allocation.event_ids[key] = self._reserve(owner, _DERIVED_MASKS, loaded, allocation)
            yield allocation
        finally:
            if not allocation.committed:
                for value in allocation.new_ids:
                    self._used.pop(value, None)
                for name in allocation.new_names:
                    self._names.pop(name, None)

    def check_bank(self, allocation: Allocation, data: bytes) -> None:
        """Prevent idempotent LoadBank from silently keeping different bytes."""
        previous = self._banks.get(allocation.owner)
        if previous is not None and previous != sha256(data).hexdigest():
            raise ValueError(f"{allocation.owner} is already loaded with different audio or settings. Restart the game")

    def commit(self, allocation: Allocation, data: bytes) -> None:
        """Retain reservations once the engine has loaded the bank."""
        self._banks[allocation.owner] = sha256(data).hexdigest()
        allocation.committed = True

    def remember_event(self, event: UObject) -> None:
        """Keep the event object available to named manifest consumers."""
        self._events[int(event.ShortId) & _MASK] = event

    def get_event(self, name: str, namespace: str, bank_name: str) -> UObject:
        """Find an event previously added with an automatic string key."""
        identifier(name, "sound name")
        scope = _scope(bank_name, namespace, {name: "3d"})
        owner = f"event:{scope}/name:{name.casefold()}"
        try:
            return self._events[self._names[owner]]
        except KeyError as exc:
            raise KeyError(f"no added event {namespace}/{bank_name}/{name} in this session") from exc

    def track_objects(self, objects: list[UObject]) -> None:
        """Recognize our own Unreal objects during later collision scans."""
        for obj in objects:
            value = int(obj.ShortId) & _MASK
            self._objects.setdefault(value, set()).add(object_path(obj))


if "registry" not in globals():
    registry = IdRegistry()
