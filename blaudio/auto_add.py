import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Any, cast

from unrealsdk import logging

from blaudio.api import POSITIONING_MODES, Positioning
from blaudio.ids import identifier
from blaudio.objects import object_path
from blaudio.runtime import add

ADD_SOUNDS = Path(__file__).parent / "add_sounds"


@dataclass(frozen=True)
class AddManifest:
    """Validated arguments for one call to `add`, plus the source path."""

    namespace: str
    bank_name: str
    sounds: dict[str, Path]
    positioning: dict[str, Positioning]
    manifest_path: Path


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

    sounds: dict[str, Path] = {}
    positioning: dict[str, Positioning] = {}
    seen: set[str] = set()
    for index, raw in enumerate(entries):
        label = f"[[sound]] #{index}"
        entry = _table(raw, label, {"name", "sound"}, {"positioning"})
        name = identifier(entry["name"], f"{label} name")
        if name.casefold() in seen:
            raise ValueError(f"{label}: duplicate sound name {name!r}")
        seen.add(name.casefold())
        mode = entry.get("positioning", "3d")
        if not isinstance(mode, str) or mode not in POSITIONING_MODES:
            raise ValueError(f"{label}: positioning must be one of {', '.join(POSITIONING_MODES)}, got {mode!r}")
        sounds[name] = _sound_path(file, entry["sound"], label)
        positioning[name] = cast(Positioning, mode)
    return AddManifest(namespace, bank_name, sounds, positioning, file)


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
