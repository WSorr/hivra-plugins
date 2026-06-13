#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

python3 scripts/validate_plugins.py
python3 scripts/validate_catalog.py
cargo test --workspace

echo "plugin review passed"
