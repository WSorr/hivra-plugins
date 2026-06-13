#!/usr/bin/env python3
import json
import re
import sys
from pathlib import Path


def fail(message: str) -> None:
    print(f"plugin validation failed: {message}", file=sys.stderr)
    raise SystemExit(1)


root = Path(__file__).resolve().parent.parent
plugin_dirs = sorted(path for path in (root / "plugins").iterdir() if path.is_dir())
if not plugin_dirs:
    fail("no plugins found")

seen_ids: set[str] = set()
allowed_capabilities = {
    "consensus_guard.read",
    "exchange.read.bingx.market",
    "exchange.trade.bingx.futures",
}

for plugin_dir in plugin_dirs:
    manifest_path = plugin_dir / "manifest.json"
    cargo_path = plugin_dir / "Cargo.toml"
    source_path = plugin_dir / "src" / "lib.rs"
    if not manifest_path.is_file() or not cargo_path.is_file() or not source_path.is_file():
        fail(f"{plugin_dir.name}: plugin unit is incomplete")

    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if manifest.get("schema") != "hivra.plugin.manifest":
        fail(f"{plugin_dir.name}: invalid manifest schema")
    if manifest.get("version") != 1:
        fail(f"{plugin_dir.name}: invalid manifest version")

    plugin_id = str(manifest.get("plugin_id", "")).strip()
    if not re.fullmatch(r"hivra\.contract\.[a-z0-9.-]+\.v[0-9]+", plugin_id):
        fail(f"{plugin_dir.name}: invalid plugin_id")
    if plugin_id in seen_ids:
        fail(f"{plugin_dir.name}: duplicate plugin_id")
    seen_ids.add(plugin_id)

    release_version = str(manifest.get("release_version", "")).strip()
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?", release_version):
        fail(f"{plugin_dir.name}: invalid release_version")

    contract_kind = str(manifest.get("contract", {}).get("kind", "")).strip()
    if not contract_kind:
        fail(f"{plugin_dir.name}: contract.kind is required")

    runtime = manifest.get("runtime", {})
    if runtime.get("abi") != "hivra_host_abi_v1":
        fail(f"{plugin_dir.name}: runtime.abi must be hivra_host_abi_v1")
    if runtime.get("entry_export") != "hivra_entry_v1":
        fail(f"{plugin_dir.name}: runtime.entry_export must be hivra_entry_v1")
    if runtime.get("module_path") != "plugin/module.wasm":
        fail(f"{plugin_dir.name}: runtime.module_path must be plugin/module.wasm")

    capabilities = manifest.get("capabilities")
    if not isinstance(capabilities, list) or capabilities != sorted(set(capabilities)):
        fail(f"{plugin_dir.name}: capabilities must be sorted and unique")
    unknown = set(capabilities) - allowed_capabilities
    if unknown:
        fail(f"{plugin_dir.name}: unknown capabilities: {sorted(unknown)}")

    source = source_path.read_text(encoding="utf-8")
    if plugin_id not in source:
        fail(f"{plugin_dir.name}: source plugin identity differs from manifest")
    if "hivra_plugin_abi_version" not in source or "hivra_entry_v1" not in source:
        fail(f"{plugin_dir.name}: required ABI exports are missing")

print(f"plugin validation passed: {len(plugin_dirs)} plugins")
