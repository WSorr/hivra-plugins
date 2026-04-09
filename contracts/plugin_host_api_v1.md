# Plugin Host API v1 (Pre-WASM Execution)

Canonical host contract currently used by Hivra plugin integration.

Source of truth for runtime behavior remains main Hivra repository. This copy is
kept here so plugin development/release can be versioned independently.

## Scope

- No wasm bytecode execution.
- Explicit API boundary for plugin calls.
- Guard-first behavior:
  - pair-scoped calls are blocked when consensus is not signable.

## Supported Contracts (v1)

- `hivra.contract.temperature-li.tomorrow.v1`
  - method: `settle_temperature_tomorrow`
- `hivra.contract.bingx-trading.v1`
  - method: `place_bingx_spot_order_intent`
- `hivra.contract.capsule-chat.v1`
  - method: `post_capsule_chat_message`

## Request Shape

```json
{
  "schema_version": 1,
  "plugin_id": "hivra.contract.bingx-trading.v1",
  "method": "place_bingx_spot_order_intent",
  "args": {
    "peer_hex": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "client_order_id": "ord-zone-1",
    "symbol": "BTC-USDT",
    "side": "buy",
    "order_type": "limit",
    "quantity_decimal": "0.02",
    "time_in_force": "GTC",
    "entry_mode": "zone_pending",
    "zone_side": "buyside",
    "zone_low_decimal": "58000",
    "zone_high_decimal": "60000",
    "zone_price_rule": "zone_mid",
    "trigger_price_decimal": "58900",
    "stop_loss_decimal": "57500",
    "take_profit_decimal": "62000",
    "created_at_utc": "2026-04-09T10:00:00Z",
    "strategy_tag": "zone-demo"
  }
}
```

## Response Shape

- `status`: `executed | blocked | rejected`
- `result`: present only for `executed`
- `blocking_facts`: present for `blocked`
- `error_code`/`error_message`: present for `rejected`
- `canonical_json` + `response_hash_hex`:
  - deterministic for identical request + runtime inputs

## Error Codes

- `invalid_schema_version`
- `unsupported_plugin`
- `unsupported_method`
- `invalid_args`

