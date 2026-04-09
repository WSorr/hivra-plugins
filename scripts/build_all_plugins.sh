#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

for plugin_dir in "$ROOT_DIR"/plugins/*; do
  [[ -d "$plugin_dir" ]] || continue
  if [[ -f "$plugin_dir/Cargo.toml" && -f "$plugin_dir/manifest.json" ]]; then
    plugin_name="$(basename "$plugin_dir")"
    "$ROOT_DIR/scripts/build_plugin_zip.sh" "$plugin_name"
  fi
done

echo "All plugin packages built in $ROOT_DIR/dist/plugins"

