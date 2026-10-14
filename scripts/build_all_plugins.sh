#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="$ROOT_DIR/dist/plugins"

# A release candidate must contain only packages from this source tree. Remove
# stale generated zips so retired or renamed plugins cannot leak into signing.
mkdir -p "$OUT_DIR"
find "$OUT_DIR" -maxdepth 1 -type f -name '*.zip' -delete

for plugin_dir in "$ROOT_DIR"/plugins/*; do
  [[ -d "$plugin_dir" ]] || continue
  if [[ -f "$plugin_dir/Cargo.toml" && -f "$plugin_dir/manifest.json" ]]; then
    plugin_name="$(basename "$plugin_dir")"
    "$ROOT_DIR/scripts/build_plugin_zip.sh" "$plugin_name"
  fi
done

echo "All plugin packages built in $ROOT_DIR/dist/plugins"

