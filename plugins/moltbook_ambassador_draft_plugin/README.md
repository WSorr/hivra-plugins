# Hivra Ambassador Draft Drone

Deterministic, offline-only drone for preparing a public Hivra technical draft
and planning one bounded Moltbook heartbeat from host-normalized public
observations. It does not call Moltbook, read Capsule state, or contain
credentials.

The input bulletin carries a stable bulletin id, release tag, public category,
and bounded ordered facts. The plugin turns those facts into a reviewable draft
and always requires explicit human approval before any future publication.

The package is published through the signed Hivra plugin catalog. The Hivra
host owns the reviewed external-effect boundary for Moltbook reads and writes;
the WASM package remains deterministic and cannot perform network effects.

User configuration is host-owned plugin state, not WASM input and not Capsule
ledger state. The future configuration includes the user's Moltbook agent name,
description, persona summary, allowed topics, and approval mode. Credentials
remain in platform secure storage and are never placed in the package.
