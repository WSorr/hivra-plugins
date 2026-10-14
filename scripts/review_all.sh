#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

python3 scripts/validate_plugins.py
cargo fmt --all -- --check
cargo test --workspace --locked
./scripts/build_all_plugins.sh
python3 scripts/validate_catalog.py --self-test

echo "plugin review passed"
