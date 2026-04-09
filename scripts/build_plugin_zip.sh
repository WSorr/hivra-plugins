#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PLUGIN_NAME="${1:-}"

if [[ -z "$PLUGIN_NAME" ]]; then
  echo "usage: $0 <plugin_dir_name>"
  exit 1
fi

PLUGIN_DIR="$ROOT_DIR/plugins/$PLUGIN_NAME"
MANIFEST_PATH="$PLUGIN_DIR/manifest.json"

if [[ ! -d "$PLUGIN_DIR" ]]; then
  echo "plugin directory not found: $PLUGIN_DIR"
  exit 1
fi

if [[ ! -f "$MANIFEST_PATH" ]]; then
  echo "manifest not found: $MANIFEST_PATH"
  exit 1
fi

WASM_MODULE="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["wasm_module"])' "$MANIFEST_PATH")"
PLUGIN_ID="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["plugin_id"])' "$MANIFEST_PATH")"
PLUGIN_VERSION="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["version"])' "$MANIFEST_PATH")"

echo "Building $PLUGIN_NAME ($PLUGIN_ID@$PLUGIN_VERSION)"

cargo build \
  --manifest-path "$PLUGIN_DIR/Cargo.toml" \
  --target wasm32-unknown-unknown \
  --release

WASM_PATH="$ROOT_DIR/target/wasm32-unknown-unknown/release/$WASM_MODULE"
if [[ ! -f "$WASM_PATH" ]]; then
  echo "wasm output not found: $WASM_PATH"
  exit 1
fi

OUT_DIR="$ROOT_DIR/dist/plugins"
TMP_DIR="$ROOT_DIR/.tmp/$PLUGIN_NAME"
ZIP_NAME="${PLUGIN_NAME}-${PLUGIN_VERSION}.zip"

rm -rf "$TMP_DIR"
mkdir -p "$TMP_DIR/plugin" "$OUT_DIR"

cp "$MANIFEST_PATH" "$TMP_DIR/plugin/manifest.json"
cp "$WASM_PATH" "$TMP_DIR/plugin/module.wasm"

(cd "$TMP_DIR" && zip -qr "$OUT_DIR/$ZIP_NAME" plugin)
echo "Built: $OUT_DIR/$ZIP_NAME"

