# BingX Futures Test Plugin

External deterministic contract plugin for:

- `plugin_id`: `hivra.contract.bingx-futures-trading.v1`
- `method`: `place_bingx_futures_order_intent`

## Behavior

- validates and normalizes futures order intent parameters
- supports `entry_mode=direct` and `entry_mode=zone_pending`
- produces canonical JSON + SHA-256 `intent_hash_hex`
- no live order execution in this repository

## Determinism

- no network calls
- no randomness
- no local-time derived fields in evaluation
- same input => byte-identical canonical JSON and hash

## ABI exports

- `hivra_plugin_abi_version() -> u32`
- `hivra_plugin_contract_id() -> u32`
- `hivra_bingx_parse_side_code(...) -> u32`

Rust API:

- `evaluate(...)`
- `evaluate_from_json(...)`
