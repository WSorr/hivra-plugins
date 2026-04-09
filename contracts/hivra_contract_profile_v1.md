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
- `plugin_id`: globally unique contract id (example: `hivra.contract.temperature-li.tomorrow.v1`)
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

Canonical payload MUST include:

- `schema_version`
- `plugin_id`
- `contract_kind`
- actor identifiers (`peer_hex` or equivalent)
- all business-critical input fields
- final decision fields (`outcome`, winner, etc)
- provenance fields (oracle/event ids when applicable)

## 4. Capability Policy

Manifest `capabilities` is required for privileged access.

Rules:

- least privilege only
- unknown capabilities => install reject
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

## 8. ABI Surface (Current Stage)

While full wasm host execution is staged, plugin packages may expose:

- metadata ABI exports (for compatibility checks)
- deterministic pure evaluation functions for future host wiring

Host API and capability checks remain authoritative.

