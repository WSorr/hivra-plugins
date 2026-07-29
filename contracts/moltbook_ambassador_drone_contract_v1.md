# Hivra Ambassador Drone Contract v1

Status: deterministic draft, heartbeat, and engagement planning

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

The current capabilities are `content.draft.prepare`, `content.feed.plan`, and
`content.engagement.plan`. None is a network capability. The host may provide a normalized, bounded
public Home/Feed observation to the heartbeat method; credentials and provider
DTOs never enter WASM.

## Heartbeat planning

`plan_moltbook_heartbeat` accepts one explicit UTC observation time, local
allowed-topic policy, bounded Home counters/actions, structured activity on
the agent's own posts, and up to 25 normalized public feed summaries. It
deterministically returns one priority:

- `review_activity` when unread account activity exists;
- `inspect_feed` when verified non-spam candidates exist;
- `idle` otherwise.

The result contains at most five candidate post ids. For `review_activity`
these ids come only from structured activity on the agent's own posts; for
`inspect_feed` they come only from verified non-spam feed candidates. The
result always marks remote content untrusted, requires human review, and sets
`publish_allowed: false`. It is a decision snapshot, not an effect, approval,
reply, vote, follow, or publication receipt.

## Engagement planning

`plan_moltbook_engagement` accepts one explicitly selected, host-normalized
post with at most 20 newest comments and the connected actor name from the
Capsule-scoped provider binding. It returns one proposal class:
`reply_draft`, `comment_draft`, `upvote_candidate`, `follow_candidate`, or
`no_action`.

The current implementation never emits `follow_candidate`: one post is not
longitudinal evidence. Own-post activity may produce `reply_draft` only for
the newest bounded comment not authored by the connected actor and not
already answered directly by that actor. A newer foreign follow-up remains
eligible. Verified, non-spam feed content may produce `comment_draft` or
`upvote_candidate`. Locked, unverified, spam-marked, or weakly evidenced
content produces `no_action`. No text is generated and no remote effect is
authorized.

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
  "reviewed_body": "Hivra keeps user-owned Capsule state local while bounded WASM plugins provide replaceable functionality.",
  "audience": "agent-developers"
}
```

Facts and `reviewed_body` are public, bounded inputs. The body may be written
manually or proposed by an AI provider, but the exact title and body must be
visible and editable before invocation. They are treated as text, never as
instructions to invoke tools or disclose secrets. The plugin does not discover
or select facts from a ledger, repository, filesystem, or network. It rejects a
mechanical newline dump of facts so a review cannot silently fall back to the
old repetitive publication format.

## Output

The plugin returns a deterministic `draft` envelope containing:

- bulletin provenance (`bulletin_id`, `release_tag`, and `category`);
- normalized title and reviewed body, preserved exactly;
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

1. provider-neutral effect states covering prepare, approval, queue, delivery,
   unresolved timeout, reconciliation, terminal receipt, and cancellation;
2. stable semantic operation ids and fake-adapter duplicate/restart fixtures;
3. provider account/claim flow and free-use confirmation;
4. Capsule/plugin/provider-scoped secure credential storage;
5. allowlisted Moltbook adapter endpoints using `www.moltbook.com`;
6. host-enforced rate limits and stop/revoke controls;
7. a dedicated Ambassador workspace with exact approval preview and receipt
   history;
8. macOS and Android offline, restart, replay, and manual approval tests.

The detailed implementation order and UI boundary are owned by
`docs/plugins/moltbook_agent_drone_design_v1.md` in the Hivra application
repository. This plugin contract owns only the WASM-facing draft boundary.

Moltbook remains external remote truth. It never becomes Core truth, a
relationship fact, a consensus input, or a backup payload.
