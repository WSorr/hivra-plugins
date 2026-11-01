# Moltbook 0.8.0

An installable stateless `plugin_workspace_v1` package. The package owns Moltbook
HTTP/protocol interpretation, fresh service history, content selection,
verification, public receipt observation and transient work. The host supplies
owner permissions, Keychain secret application, Analyst-selected `ai.infer`,
HTTP and scheduling. No service-specific host adapter or storage is used.

Connect the owner's existing API token in common plugin permissions. Configure
public GitHub owner/repository, community, editorial instructions, workflow and
interval. Open/Start/cycles reacquire profile publications and comments from
Moltbook. Prepare without sending is available. Start permits the chosen complete
workflow; the host does not require per-post approvals. Full revocation blocks
new dispatches but does not delete already accepted service content.

Recent profile history is not a lifetime archive. Unknown creation is observed
once and is not blindly replayed in the same action. With no checkpoint or
service create-idempotency key, a hidden accepted item after process exit cannot
guarantee exactly-once behavior on the next start. Verification failures and
unconfirmed public visibility are displayed, not reported as success.

The authoritative host contract and workflow are maintained in Hivra's
`docs/plugins/plugin_host_api_v1.md` and
`docs/plugins/moltbook_engagement_lifecycle_v1.md`.
Build locally with `./scripts/build_plugin_zip.sh moltbook_ambassador_draft_plugin`.
`cargo test -p hivra_moltbook_ambassador_draft_plugin` uses synthetic services only.
No publication or release is implied by a local build.
