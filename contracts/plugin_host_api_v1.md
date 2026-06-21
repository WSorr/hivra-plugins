# Plugin Host API v1 (Pre-WASM Execution)

Canonical host contract currently used by Hivra plugin integration.

Source of truth for runtime behavior remains main Hivra repository. This copy is
kept here so plugin development/release can be versioned independently.

Contract design/profile baseline is defined in:
- `contracts/hivra_contract_profile_v1.md`

## Scope

- No wasm bytecode execution.
- Explicit API boundary for plugin calls.
- Guard-first behavior:
  - pair-scoped calls are blocked when consensus is not signable.

## Supported Contracts (v1)

- `hivra.contract.bingx-futures-trading.v1`
  - method: `place_bingx_futures_order_intent`
  - method: `rank_bingx_futures_signals`
- `hivra.contract.capsule-chat.v1`
  - method: `post_capsule_chat_message`

## Request Shape

```json
{
  "schema_version": 1,
  "plugin_id": "hivra.contract.bingx-futures-trading.v1",
  "method": "place_bingx_futures_order_intent",
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

## BingX Futures Signal Ranking

`rank_bingx_futures_signals` ranks precomputed live-decision summaries. The
plugin does not fetch market data and does not place orders. Host/runtime must
provide deterministic candidate summaries produced from the current TVH/live
decision pipeline.

Request:

```json
{
  "schema_version": 1,
  "plugin_id": "hivra.contract.bingx-futures-trading.v1",
  "method": "rank_bingx_futures_signals",
  "args": {
    "schema_version": 1,
    "plugin_id": "hivra.contract.bingx-futures-trading.v1",
    "candidates": [
      {
        "symbol": "SOL-USDT",
        "can_prepare_intent": true,
        "decision": "short",
        "side": "sell",
        "zone_low_decimal": "89",
        "zone_high_decimal": "91",
        "trend_gate_code": "ok",
        "zone_anchor_source": "liquidation",
        "zone_anchor_executable": true,
        "zone_anchor_lifecycle": "fresh",
        "trend_4h": "bear",
        "trend_1d": "bear",
        "live_decision_hash_hex": "2222222222222222222222222222222222222222222222222222222222222222",
        "failed_reason_codes": []
      }
    ]
  }
}
```

Response result:

```json
{
  "entries": [
    {
      "symbol": "SOL-USDT",
      "bucket": "ready",
      "score": 10800,
      "decision": "short",
      "side": "sell",
      "zone_low_decimal": "89",
      "zone_high_decimal": "91",
      "trend_gate_code": "ok",
      "can_prepare_intent": true,
      "live_decision_hash_hex": "2222222222222222222222222222222222222222222222222222222222222222",
      "failed_reason_codes": []
    }
  ],
  "canonical_json": "...",
  "scan_hash_hex": "64 lowercase hex chars"
}
```

Ranking buckets are ordered:

1. `ready`
2. `near`
3. `blocked`
4. `no_signal`
5. `error`

Within a bucket, entries are sorted by deterministic score descending, then by
symbol ascending.

## Error Codes

- `invalid_schema_version`
- `unsupported_plugin`
- `unsupported_method`
- `invalid_args`
