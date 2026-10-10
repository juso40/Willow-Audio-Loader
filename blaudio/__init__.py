from argparse import Namespace

from mods_base import Library, build_mod, command
from unrealsdk.logging import error, misc

from blaudio import api
from blaudio.api import (
    POSITIONING_MODES,
    Positioning,
    craft_bank,
    get_id_from_string,
    load_bank,
    post_event,
    probe,
    register_game_object,
    set_active_listeners,
    to_wem,
    unload_bank,
)
from blaudio.auto_add import apply_manifest as apply_add_manifest
from blaudio.auto_add import load_manifests as load_add_manifests
from blaudio.objects import create_bank, create_event
from blaudio.runtime import AddResult, add, get_added_event, play, play_id, replace, replace_bytes

__all__ = [
    "POSITIONING_MODES",
    "AddResult",
    "Positioning",
    "add",
    "craft_bank",
    "create_bank",
    "create_event",
    "get_added_event",
    "get_id_from_string",
    "load_bank",
    "play",
    "play_id",
    "post_event",
    "probe",
    "register_game_object",
    "replace",
    "replace_bytes",
    "set_active_listeners",
    "to_wem",
    "unload_bank",
]


@command("blaudio", description="blaudio probe [pck]")
def _command(args: Namespace) -> None:
    if args.subcommand == "probe":
        print(api.probe(args.target or "Audio_Streaming.pck"))



_command.add_argument(
    "subcommand",
    choices=["probe"],
    help="subcommand",
)
_command.add_argument(
    "target",
    nargs="?",
    default=None,
    help="probe: archive to dump (bare names resolve to WillowGame/CookedPCConsole). "
)


def _enable() -> None:
    misc("blaudio: enable: loading manifest sound banks")
    for manifest in load_add_manifests():
        try:
            apply_add_manifest(manifest)
        except Exception as exc:  # noqa: BLE001
            error(f"blaudio: {manifest.manifest_path.name}: failed to add sounds: {exc}")


_enable()

build_mod(
    cls=Library,
    commands=[_command],
)
