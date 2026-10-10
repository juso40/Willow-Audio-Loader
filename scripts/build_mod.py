"""Build the release Windows extension and type stub into blaudio/."""

import os
import shutil
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
NATIVE = ROOT / "native"
TARGET = "i686-pc-windows-gnullvm"


def main() -> None:
    environment = dict(os.environ)
    for name in (
        "PYO3_BUILD_EXTENSION_MODULE",
        "PYO3_CROSS",
        "PYO3_CROSS_PYTHON_VERSION",
        "PYO3_CROSS_LIB_DIR",
    ):
        environment.pop(name, None)

    subprocess.run(
        ["cargo", "build", "--release", "-p", "blaudio", "--lib", "--target", TARGET],
        cwd=NATIVE,
        env=environment | {
            "PYO3_CROSS": "1",
            "PYO3_CROSS_PYTHON_VERSION": "3.14",
            "PYO3_BUILD_EXTENSION_MODULE": "1",
        },
        check=True,
    )
    subprocess.run(
        ["cargo", "run", "--release", "-p", "blaudio", "--bin", "stub_gen"],
        cwd=NATIVE,
        env=environment,
        check=True,
    )
    shutil.copy2(
        NATIVE / "target" / TARGET / "release" / "_blaudio.dll",
        ROOT / "blaudio" / "_blaudio.pyd",
    )


if __name__ == "__main__":
    main()
