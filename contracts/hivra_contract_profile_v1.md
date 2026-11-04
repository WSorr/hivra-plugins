# Hivra Contract Profile v1

Unified contract standard for external Hivra plugins.

This profile is designed for:

- deterministic replay
- strict downward dependencies
- explicit and auditable capabilities
- fail-closed execution

## 1. Identity and Versioning

Each contract package must define:

- `schema`: `hivra.plugin.manifest`
- `version`: `1` (manifest schema version)
- `plugin_id`: globally unique contract id (example: `hivra.contract.capsule-chat.v1`)
- `release_version`: package release version (example: `0.1.0`)
- `contract.kind`: semantic contract family id

`plugin_id` versioning is semantic API versioning.
`release_version` is package lifecycle/versioning.

## 2. Determinism Rules (Non-Negotiable)

Contract evaluation MUST NOT depend on:

- local wall-clock time
- randomness
- external network calls
- process-global mutable state

Contract output MUST be a pure function of explicit input payload.

## 3. Canonical Input/Output

Every contract evaluation must produce:

- canonical JSON payload with stable field names/order semantics
- deterministic hash of canonical JSON (`sha256_hex`)

The selected contract owns its input/output schema. Chat binds peer, client
message identity, text and creation time in canonical output. Common workspaces
receive explicit owner settings, observation time, state/context and previous
request result, then return state/context, view and at most one capability
request. A solo workspace need not invent a peer, winner or settlement DTO.

External observations and host time enter as explicit input; WASM does not read
the network or wall clock itself. Evaluation determinism does not make a remote
service or inference provider deterministic or establish exactly-once effects.

## 4. Capability Policy

Manifest `capabilities` is required for privileged access.

Rules:

- least privilege only
- unknown capabilities => install reject
- readable retired metadata does not authorize activation or invocation
- capability names are stable lowercase ids
- capability evaluation is host-controlled, not plugin-controlled

## 5. Fail-Closed Validation

On invalid input, plugin MUST return rejected/error state.
No best-effort partial execution for settlement logic.

Validation includes:

- schema and plugin identity checks
- type checks
- enum domain checks
- domain invariants (location, bounds, consistency)

## 6. Replay and Idempotency

Contracts should carry replay anchors (for example `event_id`, `intent_id`).
Repeated evaluation of same canonical inputs MUST return byte-identical outputs and hash.

## 7. Test Requirements

Minimum required tests:

- deterministic repeatability (same input => same output/hash)
- invariant checks and fail-closed cases
- golden vectors for at least one happy path and one draw/edge path

## 8. ABI Surface

Installed packages are ZIPs containing `plugin/manifest.json` and
`plugin/module.wasm`. They declare `runtime.abi = hivra_host_abi_v2` and export:

- `hivra_alloc_v1(len: u32) -> u32`
- `hivra_evaluate_v1(ptr: u32, len: u32) -> u64`
- `hivra_dealloc_v1(ptr: u32, len: u32)`

The packed result is `(output_ptr << 32) | output_len`. Evaluation consumes and
returns bounded UTF-8 JSON. Modules import no host functions; technical requests
return through the common continuation protocol. Metadata exports alone do not
make an installed package executable.

The operation catalog, permissions, bounds and maintained specialized contract
are owned by Hivra's `docs/plugins/plugin_host_api_v1.md`; optional abilities are
discovered with `api.describe`. Product changes within supported abilities do
not require a new host adapter or Capsule build.
