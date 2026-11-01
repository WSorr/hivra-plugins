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

Plugin state, service protocol, parsing, decisions and workflow remain private
to WASM. The host owns sandbox isolation, owner-grant enforcement, network/TLS,
protected secret application, scheduling and optional opaque storage. It never
normalizes service receipts or repeats a package's business policy.

Workspace invocations use `api_version: 2` over the existing ABI. Public
operations are `api.describe`, `runtime.yield`, `http.request`,
`runtime.wait`, `storage.read`, `storage.write`, `ai.infer`. They contain
no service names. Request fields, permission descriptors, authentication,
outcomes and bounds have one canonical owner in the main contract linked above.
Each output has opaque `state`/`context`, a generic `view`, and zero or one
requests containing `kind`, correlation `id` and package-selected `continuation`.

One owner grant covers destination, methods, selected Keychain references and
optional inference. Start reuses that grant. Permissions do not require a
schedule. Credentials are connected by the owner screen, not a package
acceptance operation. A stateless package needs no storage capability; null
state is transient and no workspace checkpoint is read or written. The host checks the installed package, current owner/grant
and executor again at dispatch, including after queued inference. Protected
values never reach WASM. A response is raw bounded HTTP evidence; non-dispatch
and uncertainty are distinct and the host never retries. Necessary private
pending intent belongs to the package and may use its optional opaque store. Revocation cannot undo an
external service's already admitted request.

Old named workspace requests are rejected. Old local Jack test-session journals
are not imported in this pass; the owner excluded that migration. Jack 0.2.0
requires a new workspace session (normal reinstall for an old local test
workspace with explicit saved-data deletion), and rejects an incompatible
checkpoint without overwriting it. Normal workspace uninstall retains opaque
checkpoint/data for recovery, deletes keys, and invalidates execution access;
erasing saved data is a separate owner choice shown by the shell. A future
session migration must define preservation of unresolved operation identities.
This does not remove restart recovery for new protocol-2 attempts.

## Supported Contracts (v1)

- `hivra.contract.capsule-chat.v1`
  - method: `post_capsule_chat_message`
- Moltbook 0.8.0 uses `plugin_workspace_v1`, no storage capability, and no named
  service methods; history is read from the service on every start.
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
