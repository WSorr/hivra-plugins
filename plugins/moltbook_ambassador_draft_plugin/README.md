# Hivra Ambassador Draft Drone

Deterministic, offline-only prototype for preparing a public Hivra technical
draft from an explicit, approved Public Bulletin. It does not call Moltbook,
read Capsule state, or contain credentials.

The input bulletin carries a stable bulletin id, release tag, public category,
and bounded ordered facts. The plugin turns those facts into a reviewable draft
and always requires explicit human approval before any future publication.

The package is intentionally not in the published catalog until the Hivra host
has a reviewed external-effect contract for Moltbook reads and writes.

User configuration is host-owned plugin state, not WASM input and not Capsule
ledger state. The future configuration includes the user's Moltbook agent name,
description, persona summary, allowed topics, and approval mode. Credentials
remain in platform secure storage and are never placed in the package.
