# Capsule Chat

Installed package for `hivra.contract.capsule-chat.v1`, contract kind
`capsule_chat`, method `post_capsule_chat_message`. The historical source
folder name is not the displayed product name. The manifest owns its release
version and display name.

The bounded WASM evaluator validates the peer, client message identity, text
and creation timestamp, then produces canonical JSON and its SHA-256 envelope
hash. It performs no transport I/O. Capsule owns Pair Consensus, current package
binding and the canonical durable delivery path; successful preparation alone
does not authorize a later send.

Chat retains a specialized host workspace/relationship contract rather than
using Moltbook and Jack's generic protocol-2 workspace. Its current request/result
boundary belongs to Hivra's `docs/plugins/plugin_host_api_v1.md`; delivery and
retention belong to `docs/architecture/transport-delivery-lifecycle.md`.
The package uses the same ZIP/manifest admission and ABI v2 as other installed
packages. It is an executable evaluator, not a metadata-only stub.
