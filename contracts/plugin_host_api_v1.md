# Plugin Host API v1 (WASM Execution Boundary)

Package-facing summary of the host contract used by Hivra external plugins.

The canonical runtime contract is maintained in the main Hivra repository at
`docs/plugins/plugin_host_api_v1.md`. This copy is intentionally limited to
stable package-facing rules so plugin releases can be versioned independently.

Contract design/profile baseline is defined in:
- `contracts/hivra_contract_profile_v1.md`

## Scope

- External packages execute as bounded WASM through `wasmi_v1`.
- Packages use runtime ABI `hivra_host_abi_v2` and export
  `hivra_evaluate_v1`.
- The host validates the manifest, package digest, requested capability and
  canonical output before any host-owned effect is considered.
- Pair-scoped calls are blocked when consensus is not signable.

Plugin state and strategy decisions remain private to WASM. The host owns
authorization, credentials, persistence, normalized provider evidence and
permitted effects. A package replacement must use the same host contract
without requiring Capsule or Core changes.

## Supported Contracts (v1)

- `hivra.contract.capsule-chat.v1`
  - method: `post_capsule_chat_message`
- `hivra.contract.moltbook-ambassador.v1`
  - draft, heartbeat and bounded engagement planning methods
- `plugin_workspace_v1`
  - bounded package-owned workspace state and host-mediated requests

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
