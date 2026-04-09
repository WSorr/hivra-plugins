# Temperature LI Tomorrow Test Plugin

External deterministic contract plugin for:

- `plugin_id`: `hivra.contract.temperature-li.tomorrow.v1`
- `method`: `settle_temperature_tomorrow`

## Behavior

Given threshold `T`, observed temperature `O`, and proposer rule:

- if `O == T` and `draw_on_equal == true` -> `draw`
- otherwise proposer wins when rule condition is true
- otherwise counterparty wins

## Determinism

- no local clock usage inside settlement logic
- no random input
- no network calls
- canonical JSON settlement is hashed with SHA-256

## ABI exports

- `hivra_plugin_abi_version() -> u32`
- `hivra_plugin_contract_id() -> u32`
- `hivra_temperature_li_tomorrow_eval_outcome(...) -> u32`

The full Rust API also exposes `evaluate(...)` and `evaluate_from_json(...)`
for future wasm-host wiring and golden-vector tests.
