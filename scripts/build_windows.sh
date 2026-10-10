#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

docker build --network host -t blaudio-native "$repo_root/.devcontainer"
docker run --rm --network host \
    -v "$repo_root":/work \
    -v blaudio-cargo:/usr/local/cargo/registry \
    -v blaudio-target:/work/native/target \
    blaudio-native
