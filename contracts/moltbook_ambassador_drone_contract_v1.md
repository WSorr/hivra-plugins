# Hivra Ambassador Drone Contract v1

Status: draft-only prototype

Plugin id: `hivra.contract.moltbook-ambassador.v1`

This contract prepares a public technical draft from an explicit `Public
Bulletin` for a human to review. It does not connect to Moltbook, read a
Capsule, access a ledger, use an AI provider, store credentials, or publish
remote content.

## Contract boundary

```text
explicit Public Bulletin
  -> Ambassador Drone (WASM, deterministic)
  -> validated public draft + hash
```

A Public Bulletin is a separate, intentionally published projection of public
development facts. It is not the ledger, a contact card, a backup, or an
implicit permission to publish. The producer must provide a stable
`bulletin_id`; the host may later attach a signature and release metadata.

The current capability is `content.draft.prepare`. It is intentionally not a
network capability. Moltbook reads and writes require a future host external-
effect contract with scoped credentials, rate limits, receipts, retries, and
explicit approval.

## Input

```json
{
  "schema_version": 1,
  "plugin_id": "hivra.contract.moltbook-ambassador.v1",
  "bulletin_id": "release-v1.0.3-test14",
  "release_tag": "v1.0.3-test14",
  "category": "release",
  "facts": [
    "Hivra is a local-first runtime for user-owned Capsules.",
    "The release includes bounded WASM plugin execution."
  ],
  "title_hint": "A local-first runtime for user-owned Capsules",
  "audience": "agent-developers"
}
```

Facts are public, bounded input. They are treated as text, never as
instructions to invoke tools or disclose secrets. The plugin does not discover
or select facts from a ledger, repository, filesystem, or network.

## Output

The plugin returns a deterministic `draft` envelope containing:

- bulletin provenance (`bulletin_id`, `release_tag`, and `category`);
- normalized title and body assembled from the ordered facts;
- `approval_required: true`;
- safety flags;
- a SHA-256 hash of the canonical draft.

The output is not a publication receipt and must not be interpreted as remote
success.

## Ambassador Configuration v1

The user-facing identity and policy are one host-owned plugin-state document.
They are not manifest fields, ledger events, Capsule identity, or WASM secrets.

```json
{
  "schema_version": 1,
  "plugin_id": "hivra.contract.moltbook-ambassador.v1",
  "agent_name": "hivra_ambassador",
  "agent_description": "Technical ambassador for Hivra.",
  "persona_summary": "Explain public Hivra development clearly and factually.",
  "allowed_topics": ["hivra-development", "capsule-runtime", "wasm-drones"],
  "approval_mode": "assisted",
  "enabled": true
}
```

Rules:

- `agent_name` and `agent_description` are user-configurable Moltbook profile
  values; they do not rename the Capsule or the plugin package.
- `persona_summary` and `allowed_topics` are local policy, not public facts and
  not instructions received from Moltbook.
- `approval_mode` is fail-closed: v1 permits `draft` and `assisted`; remote
  autonomous modes are not valid until the external-effect contract exists.
- `enabled` is a local stop control and does not delete the remote account.
- API keys, claim tokens, Capsule seeds, transport keys, ledger data, and
  private contact data are forbidden in this document.
- In this draft-only version the document is scoped by `(capsule_root,
  plugin_id)` and must be isolated from other plugin instances. A future
  external-account binding may extend the scope with `provider_id` and
  `provider_account_id`.

The current WASM draft method does not consume this configuration yet: it
accepts only an explicit Public Bulletin. The host will apply this policy when
the future registration, preview, and external-publication ports are added.

## Mandatory safety rules

- Never include seed phrases, API keys, credentials, ledger contents, contact
  cards, private Capsule data, or local filesystem paths.
- Reject drafts that promote cryptocurrency, trading signals, financial
  products, spam, impersonation, or medical/legal advice.
- Do not infer identity, reputation, ownership, or authorization from source
  text.
- Keep the first user-facing mode assisted: exact draft preview and explicit
  approval are required for every future remote write.

## Future Moltbook adapter gate

Implementation of remote access is blocked until all of the following exist:

1. provider account/claim flow and free-use confirmation;
2. Capsule/plugin/provider-scoped secure credential storage;
3. allowlisted Moltbook adapter endpoints using `www.moltbook.com`;
4. durable external-effect operation ids and receipt reconciliation;
5. host-enforced rate limits and stop/revoke controls;
6. macOS and Android offline, restart, replay, and manual approval tests.

Moltbook remains external remote truth. It never becomes Core truth, a
relationship fact, a consensus input, or a backup payload.
