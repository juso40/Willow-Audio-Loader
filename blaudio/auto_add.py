import tomllib
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, cast

from unrealsdk import logging

from .api import POSITIONING_MODES, Positioning, gain_value
from .ids import identifier, stop_event_names
from .objects import object_path
from .runtime import add
from .sources import ROUTING_MODES, Routing, SoundPool, SoundSource, Switch, source_spec

ADD_SOUNDS = Path(__file__).parent / "add_sounds"


@dataclass(frozen=True)
class AddManifest:
    """Validated arguments for one call to `add`, plus the source path."""

    namespace: str
    bank_name: str
    sounds: dict[str, SoundSource]
    positioning: dict[str, Positioning]
    looping: dict[str, bool]
    stop_events: dict[str, str]
    routing: dict[str, Routing]
    manifest_path: Path
    gain: dict[str, float] = field(default_factory=dict)


def load_manifests() -> list[AddManifest]:
    """Scan recursively in sorted order, skipping invalid manifests."""
    ADD_SOUNDS.mkdir(exist_ok=True)
    logging.info(f"blaudio: loading add-sound manifests from {ADD_SOUNDS.absolute()}")
    manifests: list[AddManifest] = []
    for file in sorted(ADD_SOUNDS.rglob("*.toml")):
        if manifest := load_manifest(file):
            manifests.append(manifest)
    logging.misc(f"blaudio: {len(manifests)} manifest(s) ready")
    return manifests


def load_manifest(file: Path) -> AddManifest | None:
    """Read and validate a whole manifest before adding any sounds."""
    logging.misc(f"blaudio: reading manifest {file}")
    try:
        with file.open("rb") as stream:
            return _parse_manifest(file, tomllib.load(stream))
    except (OSError, ValueError) as exc:
        logging.error(f"blaudio: {file}: {exc}")
        return None


def apply_manifest(manifest: AddManifest) -> int:
    """Delegate allocation, bank loading, and object creation to `add`."""
    result = add(
        manifest.sounds,
        namespace=manifest.namespace,
        bank_name=manifest.bank_name,
        positioning=manifest.positioning,
        looping=manifest.looping,
        stop_events=manifest.stop_events,
        routing=manifest.routing,
        gain=manifest.gain,
    )
    logging.info(f"blaudio: loaded bank 0x{result.bank_id:08x} ({len(manifest.sounds)} sound(s))")
    ids = result.event_ids
    for name, event in result.events.items():
        logging.misc(f"blaudio:   {name!r}, event 0x{ids[name]:08x} -> {object_path(event)}")
    return result.bank_id


def _parse_manifest(file: Path, data: dict[str, Any]) -> AddManifest:
    """The single supported schema. Validation errors raise `ValueError`."""
    _table(data, "manifest", {"namespace", "bank_name", "sound"})
    namespace = identifier(data["namespace"], "namespace")
    bank_name = identifier(data["bank_name"], "bank_name")
    entries = data["sound"]
    if not isinstance(entries, list) or not entries:
        raise ValueError("expected at least one [[sound]] table")

    sounds: dict[str, SoundSource] = {}
    positioning: dict[str, Positioning] = {}
    looping: dict[str, bool] = {}
    stop_events: dict[str, str] = {}
    routing: dict[str, Routing] = {}
    gains: dict[str, float] = {}
    seen: set[str] = set()
    for index, raw in enumerate(entries):
        label = f"[[sound]] #{index}"
        entry = _table(raw, label, {"name", "sound"}, {"positioning", "loop", "stop_event", "routing", "gain"})
        name = identifier(entry["name"], f"{label} name")
        if name.casefold() in seen:
            raise ValueError(f"{label}: duplicate sound name {name!r}")
        seen.add(name.casefold())
        mode = entry.get("positioning", "3d")
        if not isinstance(mode, str) or mode not in POSITIONING_MODES:
            raise ValueError(f"{label}: positioning must be one of {', '.join(POSITIONING_MODES)}, got {mode!r}")
        loop = entry.get("loop", False)
        if not isinstance(loop, bool):
            raise ValueError(f"{label}: loop must be a boolean, got {loop!r}")
        route = entry.get("routing", "sfx")
        if not isinstance(route, str) or route not in ROUTING_MODES:
            raise ValueError(f"{label}: routing must be one of {', '.join(ROUTING_MODES)}")
        gains[name] = gain_value(entry.get("gain", 0), f"{label} gain")
        source = _sound_source(file, entry["sound"], label)
        source_spec(source)
        sounds[name] = source
        routing[name] = cast(Routing, route)
        positioning[name] = cast(Positioning, mode)
        looping[name] = loop
        if "stop_event" in entry:
            stop_events[name] = identifier(entry["stop_event"], f"{label} stop_event")
    stops = stop_event_names(sounds, stop_events)
    return AddManifest(namespace, bank_name, sounds, positioning, looping, stops, routing, file, gains)


def _table(
    raw: object, label: str, required: set[str], optional: set[str] | frozenset[str] = frozenset()
) -> dict[str, Any]:
    """Require a table with exactly the documented required/optional keys."""
    if not isinstance(raw, dict):
        raise ValueError(f"{label}: expected a table")
    if unknown := raw.keys() - required - optional:
        raise ValueError(f"{label}: unknown keys: {', '.join(sorted(unknown))}")
    if missing := required - raw.keys():
        raise ValueError(f"{label}: missing required keys: {', '.join(sorted(missing))}")
    return raw


def _sound_source(file: Path, raw: object, label: str, depth: int = 0) -> SoundSource:
    """Decode an audio path, pool, or nested switch/state table."""
    if depth > 16:
        raise ValueError(f"{label}: sound tree exceeds maximum depth")
    if isinstance(raw, str):
        return _sound_path(file, raw, label)
    if isinstance(raw, list):
        return SoundPool([_sound_path(file, clip, label) for clip in raw])
    if isinstance(raw, dict) and "clips" in raw:
        table = _table(raw, label, {"clips"}, {"selection", "weights", "avoid_repeat", "scope"})
        if not isinstance(table["clips"], list):
            raise ValueError(f"{label}: clips must be an array of audio paths")
        return SoundPool(
            [_sound_path(file, clip, label) for clip in table["clips"]],
            selection=table.get("selection", "random"),
            weights=table.get("weights"),
            avoid_repeat=table.get("avoid_repeat"),
            scope=table.get("scope", "game_object"),
        )
    table = _table(raw, label, {"group", "branches", "default"}, {"kind"})
    if not isinstance(table["branches"], dict):
        raise ValueError(f"{label}: branches must be a table")
    branches = {}
    for state, branch in table["branches"].items():
        if isinstance(branch, dict) and "silent" in branch:
            _table(branch, label, {"silent"})
            if branch["silent"] is not True:
                raise ValueError(f"{label}: silent must be true")
            branches[state] = None
        else:
            branches[state] = _sound_source(file, branch, label, depth + 1)
    return Switch(table["group"], branches, table["default"], kind=table.get("kind", "switch"))


def _sound_path(file: Path, raw: object, label: str) -> Path:
    """Resolve an existing audio file relative to its manifest."""
    if not isinstance(raw, str) or not raw.strip():
        raise ValueError(f"{label}: sound must be a non-empty relative file path")
    path = Path(raw)
    if path.is_absolute():
        raise ValueError(f"{label}: sound must be relative to the manifest")
    path = (file.parent / path).resolve()
    if not path.is_file():
        raise ValueError(f"{label}: sound file not found: {path}")
    return path
