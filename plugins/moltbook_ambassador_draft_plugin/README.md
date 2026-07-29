# Hivra Ambassador Draft Drone

Deterministic, offline-only drone for preparing a public Hivra technical draft
and planning one bounded Moltbook heartbeat from host-normalized public
observations. It does not call Moltbook, read Capsule state, or contain
credentials.

Structured activity on the agent's own posts takes priority over general feed
inspection. The heartbeat plan returns exact post ids for review, but remains
read-only and cannot reply, vote, follow, or publish.

The `plan_moltbook_engagement` method accepts one user-selected,
host-normalized conversation plus the connected actor name. It may propose a
reply draft, comment draft, upvote candidate, or no action. Reply plans never
target a comment authored by that actor. Follow stays unavailable from a
single observation. Every proposal requires human review and denies external
effects.
The separate `prepare_moltbook_reply` method binds exact reviewed prose to the
selected post, optional parent comment, and engagement-plan hash. It remains a
pure decision and cannot publish.

The input bulletin carries a stable bulletin id, release tag, public category,
bounded supporting facts, and an explicitly reviewed title and body. The
plugin rejects a mechanical fact dump, preserves the reviewed prose in its
canonical draft, and always requires explicit human approval before any future
publication.

The package is published through the signed Hivra plugin catalog. The Hivra
host owns the reviewed external-effect boundary for Moltbook reads and writes;
the WASM package remains deterministic and cannot perform network effects.

User configuration is host-owned plugin state, not WASM input and not Capsule
ledger state. The future configuration includes the user's Moltbook agent name,
description, persona summary, allowed topics, and approval mode. Credentials
remain in platform secure storage and are never placed in the package.
