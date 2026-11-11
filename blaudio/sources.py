from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Literal

AudioSource = str | Path | bytes
Selection = Literal["random", "shuffle", "sequence"]
Scope = Literal["game_object", "global"]
Routing = Literal["sfx", "voice", "player_voice"]
ROUTING_MODES = ("sfx", "voice", "player_voice")
# Keep this derivation and the fixed id masks in sync with audio/src/craft.rs.
_NODE_STEP = 0x9E3779B9
_MAX_CLIPS = 65535
_MAX_DEPTH = 16
_MAX_NODES = 65535


@dataclass(frozen=True)
class SoundPool:
    """Choose one clip per post, natively in Wwise.

    random: weighted random, excluding the last `avoid_repeat` choices.
    shuffle: exhaust the pool before refilling it (weights influence order).
    sequence: advance in the supplied order, wrapping after the last clip.
    History is emitter-local by default. `scope="global"` shares it.
    Default repeat avoidance is one for random/shuffle, zero for sequence.
    One-clip pools always use zero. Looping repeats the selected clip, NOT
    the playlist. Continuous playlists/crossfades are not exposed here.
    """

    clips: Sequence[AudioSource]
    selection: Selection = "random"
    weights: Sequence[int] | None = None
    avoid_repeat: int | None = None
    scope: Scope = "game_object"


@dataclass(frozen=True)
class Switch:
    """Select a subtree using an existing Wwise switch or global state.

    Names use Wwise's case-insensitive hash. Nonzero numeric ids also work.
    `default` must name a declared branch. None is an explicitly silent
    branch, distinct from an unhandled value falling back to `default`.
    `kind="state"` reads a global state, NOT an emitter-local switch.
    This declares playback logic. It does not set or register engine values.
    """

    group: str | int
    branches: Mapping[str | int, SoundSource | None]
    default: str | int
    kind: Literal["switch", "state"] = "switch"


type SoundSource = AudioSource | SoundPool | Switch


def wwise_id(value: str | int) -> int:
    """Wwise FNV-1, verified against the game's FirstPerson/Player ids."""
    if isinstance(value, str):
        if not value or not value.isascii() or "\0" in value:
            raise ValueError("Wwise names must be non-empty ASCII without NUL. Use a numeric id otherwise")
        result = 2166136261
        for char in value.lower().encode("ascii"):
            result = ((result * 16777619) ^ char) & 0xFFFFFFFF
    elif isinstance(value, int) and not isinstance(value, bool) and 0 < value <= 0xFFFFFFFF:
        result = value
    else:
        raise ValueError("Wwise ids must be names or integers between 1 and 0xFFFFFFFF")
    if not result:
        raise ValueError("Wwise id zero is reserved")
    return result


def _pool_parameters(node: SoundPool) -> dict:
    if not isinstance(node.clips, (list, tuple)) or not 1 <= len(node.clips) <= _MAX_CLIPS:
        raise ValueError("SoundPool.clips must be a non-empty list/tuple of at most 65535 audio sources")
    if node.selection not in ("random", "shuffle", "sequence"):
        raise ValueError("selection must be random, shuffle, or sequence")
    if node.scope not in ("game_object", "global"):
        raise ValueError("scope must be game_object or global")
    if any(not isinstance(clip, (str, Path, bytes)) for clip in node.clips):
        raise ValueError("SoundPool entries must be audio paths/bytes, not nested selection trees")
    avoid = node.avoid_repeat
    if avoid is None:
        avoid = int(node.selection != "sequence" and len(node.clips) > 1)
    if isinstance(avoid, bool) or not isinstance(avoid, int) or not 0 <= avoid < len(node.clips):
        raise ValueError("avoid_repeat must be an integer from zero to clip_count - 1")
    weights = [50000] * len(node.clips)
    if node.weights is not None:
        if not isinstance(node.weights, (list, tuple)) or len(node.weights) != len(node.clips):
            raise ValueError("weights must contain one positive integer per clip")
        weights = list(node.weights)
        if any(isinstance(w, bool) or not isinstance(w, int) or not 0 < w <= 0xFFFFFFFF for w in weights):
            raise ValueError("weights must contain positive uint32 integers")
    # BL2's weighted draw combines two 15-bit rand() results. Larger
    # totals make the tail of the playlist unreachable, not merely rarer.
    if sum(weights) > 0x40000000:
        raise ValueError("sum of weights must not exceed 0x40000000 (Wwise's random range)")
    if node.selection == "sequence" and (node.weights is not None or avoid):
        raise ValueError("sequence does not use weights or repeat avoidance")
    return {
        "type": "pool",
        "selection": node.selection,
        "avoid_repeat": avoid,
        "global": node.scope == "global",
        "weights": weights,
    }


def source_spec(source: SoundSource) -> dict:
    """Validate and snapshot a tree without reading audio files.

    The normalized shape determines HIRC reservations and native input.
    Read its media with read_source only after every option is validated.
    """
    count = 0
    clips = 0
    active: set[int] = set()

    def visit(node: SoundSource | None, depth: int) -> dict | None:  # noqa: PLR0912 - explicit tree validation
        nonlocal count, clips
        if node is None:
            return None
        count += 1
        if depth > _MAX_DEPTH or count > _MAX_NODES:
            raise ValueError("sound tree exceeds the depth (16) or node (65535) limit")
        if isinstance(node, (str, Path, bytes)):
            if not isinstance(node, bytes) and not str(node):
                raise ValueError("audio paths must not be empty")
            clips += 1
            return {"type": "clip", "audio": node}
        if id(node) in active:
            raise ValueError("sound trees cannot contain cycles")
        active.add(id(node))
        try:
            if isinstance(node, SoundPool):
                return _pool_parameters(node) | {"clips": [visit(clip, depth + 1) for clip in node.clips]}
            if isinstance(node, Switch):
                if node.kind not in ("switch", "state"):
                    raise ValueError("Switch.kind must be switch or state")
                if not isinstance(node.branches, Mapping) or not 1 <= len(node.branches) <= _MAX_CLIPS:
                    raise ValueError("Switch.branches must be a non-empty mapping of at most 65535 branches")
                group, default = wwise_id(node.group), wwise_id(node.default)
                branches = []
                seen = set()
                for state, branch in node.branches.items():
                    value = wwise_id(state)
                    if value in seen:
                        raise ValueError("duplicate switch/state branch id (names are case-insensitive)")
                    seen.add(value)
                    branches.append((value, visit(branch, depth + 1)))
                if default not in seen:
                    raise ValueError("Switch.default must identify a declared branch")
                return {"type": "switch", "kind": node.kind, "group": group, "default": default, "branches": branches}
            raise ValueError("expected an audio path/bytes, SoundPool, or Switch")
        finally:
            active.remove(id(node))

    spec = visit(source, 0)
    if spec is None:
        raise ValueError("an event cannot have a silent root. Use an explicit silent Switch branch")
    if not clips:
        raise ValueError("sound trees must contain at least one audio clip")
    return spec


def read_source(spec: dict) -> dict:
    """Read media from a validated snapshot without revisiting caller containers."""
    if spec["type"] == "clip":
        audio = spec["audio"]
        return {"type": "clip", "audio": audio if isinstance(audio, bytes) else Path(audio).read_bytes()}
    if spec["type"] == "pool":
        return spec | {"clips": [read_source(clip) for clip in spec["clips"]]}
    return spec | {
        "branches": [(state, None if branch is None else read_source(branch)) for state, branch in spec["branches"]]
    }


def resolved_source(spec: dict) -> SoundSource:
    """Rebuild an authoring tree from a validated source snapshot."""
    if spec["type"] == "clip":
        return spec["audio"]
    if spec["type"] == "pool":
        return SoundPool(
            tuple(clip["audio"] for clip in spec["clips"]),
            selection=spec["selection"],
            weights=None if spec["selection"] == "sequence" else tuple(spec["weights"]),
            avoid_repeat=spec["avoid_repeat"],
            scope="global" if spec["global"] else "game_object",
        )
    return Switch(
        spec["group"],
        {state: None if branch is None else resolved_source(branch) for state, branch in spec["branches"]},
        spec["default"],
        kind=spec["kind"],
    )


def extra_masks(spec: dict, positioning: str) -> tuple[int, ...]:
    """Additional ids used by a tree, excluding the fixed event family."""

    def nodes(tree: dict) -> int:
        if tree["type"] == "clip":
            return 1
        if tree["type"] == "pool":
            return 1 + len(tree["clips"])
        return 1 + sum(nodes(branch) for _, branch in tree["branches"] if branch is not None)

    # Clips use the fixed sound id. Trees allocate every source
    # node from a separate indexed family. Perspective duplicates the tree,
    # but both copies reuse the same media ids.
    n = 0 if spec["type"] == "clip" else nodes(spec) * (2 if positioning == "perspective" else 1)
    return tuple((i * _NODE_STEP) & 0xFFFFFFFF for i in range(1, n + 1))
