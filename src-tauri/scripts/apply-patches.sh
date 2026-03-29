#!/usr/bin/env bash
# Creates .patches/ by copying crates from the local registry (or downloading
# from crates.io as fallback) and applying the patch files in patches/.
# Run this once after cloning, and again whenever a patch file changes.
# The resulting .patches/ directory is gitignored.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

apply_patch() {
    local name="$1" version="$2"
    local dest=".patches/${name}-${version}"

    if [[ -d "$dest" ]]; then
        echo "  $name-$version: already present, skipping"
        return
    fi

    mkdir -p .patches

    # Prefer the local Cargo registry (already extracted, fast)
    local registry_src
    registry_src=$(find "${CARGO_HOME:-$HOME/.cargo}/registry/src" \
        -maxdepth 2 -name "${name}-${version}" -type d 2>/dev/null | head -1 || true)

    if [[ -n "$registry_src" ]]; then
        echo "  $name-$version: copying from local registry..."
        cp -r "$registry_src" "$dest"
    else
        echo "  $name-$version: downloading from crates.io..."
        curl -sSL "https://static.crates.io/crates/${name}/${version}/download" \
            | tar -xz -C .patches/
    fi

    # Registry files are read-only; make them writable before patching
    chmod -R u+w "$dest"
    patch -d "$dest" -p1 < "patches/${name}.patch"
    echo "  $name-$version: patched"
}

echo "Applying crate patches..."
apply_patch ffmpeg-sys-next 7.1.3
apply_patch llama-cpp-sys-2 0.1.138
echo "Done."
