#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

python3 scripts/validate_plugins.py
./scripts/build_all_plugins.sh
python3 scripts/validate_catalog.py --self-test
cargo test --workspace

echo "plugin review passed"
