# Moltbook Ambassador Package Contract

Moltbook is an installed stateless `plugin_workspace_v1` package using common
API protocol 2 over `hivra_host_abi_v2`. Its `plugin_id` remains
`hivra.contract.moltbook-ambassador.v1`; that identity does not select a named
service handler in Capsule. The release version is owned by the manifest.

## Canonical Boundaries

The maintained host wire contract is
[Plugin Host API](https://github.com/WSorr/Hivra-App/blob/main/docs/plugins/plugin_host_api_v1.md).
The service workflow, fresh-history reconstruction, receipt checks and changed
stateless recovery guarantees have one owner:
[Moltbook Engagement Lifecycle](https://github.com/WSorr/Hivra-App/blob/main/docs/plugins/moltbook_engagement_lifecycle_v1.md).
This package summary does not define another host lifecycle.

The package owns HTTP request construction, parsing, public source selection,
news/reply/challenge prompts, verification and service-result interpretation.
The host supplies owner-granted HTTP, protected secret application, Analyst AI
inference, bounded execution and scheduling. It has no Moltbook adapter,
publication journal, normalized-feed controller or editorial approval chain.

## Owner Workflow

Connect the owner's Keychain token through the common permission screen and
choose repository, community, editorial instructions, workflow and interval.
Owner settings live in the existing permission record, not a publication store.
An authorized open/start and each cycle read current account publications and
comments from Moltbook. Prepare without sending produces a preview; Start can
authorize the configured complete workflow without per-publication approvals.

The package requests `http.request`, `ai.infer` and bounded `runtime.wait`
through package-selected continuations. Its manifest has no storage capability;
private work is transient and `state` is null. Service account facts never become
Core/Ledger facts. Full revocation prevents new dispatch but cannot undo content
already accepted by the service. Replacement needs a grant bound to the new
package digest.

## Recovery And Compatibility

Recent service history is not a lifetime archive. Unknown creation is observed
without blindly resending in the same action. If accepted content remains hidden
from history after process exit, service-only reconstruction cannot guarantee
exactly-once creation across starts. Neither a host queue nor a permanent local
archive is silently added to change this owner-selected retention model.

The former `prepare_moltbook_*`, `plan_moltbook_*` and
`authorize_moltbook_delegated_reply` host methods are retired. Legacy `content.*`
manifest names may be readable for archive recovery/removal; they grant no
execution path. Do not reintroduce the old host approval, daily-write counter,
feed checkpoint or publication-effect protocol when extending this package.

Tests exercise package semantics with synthetic HTTP/AI results. Completed
native stages and remaining live-write acceptance are recorded in the main
repository's development control; neither a local build nor this contract
qualifies publication/reply/challenge execution.
