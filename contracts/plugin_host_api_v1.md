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

- `hivra.contract.capsule-chat.v1`
  - method: `post_capsule_chat_message`

## Request Shape

```json
{
  "schema_version": 1,
  "plugin_id": "hivra.contract.capsule-chat.v1",
  "method": "post_capsule_chat_message",
  "args": {
    "peer_hex": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "client_message_id": "msg-1",
    "message_text": "hello"
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
