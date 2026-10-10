"""Bundle the mod folder into `blaudio.zip` for distribution.

The mod manager unpacks the archive into `Binaries/Win32/Plugins/sdk_mods/`,
so every entry lives under a top-level `blaudio/` folder. The bundle skips
interpreter cruft (`__pycache__`) and nested archives but includes the built
`_blaudio.pyd`. Entries are added in sorted order, so re-running without
changes reproduces the archive byte for byte.

Usage: `python make_zip.py`
"""

from __future__ import annotations

import zipfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent
MOD_DIR = REPO_ROOT / "blaudio"
ARCHIVE_SUFFIX = ".zip"
SKIP_PARTS = frozenset({"__pycache__"})


def bundle(mod_dir: Path) -> Path:
    """Zip `mod_dir` into `<name>.zip` beside it, then return the archive path."""
    archive = mod_dir.with_suffix(ARCHIVE_SUFFIX)
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as zf:
        for path in sorted(mod_dir.rglob("*")):
            if path.is_dir() or not SKIP_PARTS.isdisjoint(path.parts) or path.suffix == ARCHIVE_SUFFIX:
                continue
            zf.write(path, f"{mod_dir.name}/{path.relative_to(mod_dir).as_posix()}")
    return archive


def main() -> None:
    archive = bundle(MOD_DIR)
    print(f"wrote {archive.name} ({archive.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
