# hivra-plugins

External WASM plugins repository for Hivra.

This repository is licensed under the MIT License. See
[LICENSE](LICENSE). Individual dependencies retain their own licenses.

This repo contains plugin packages and build/release tooling only.
Core app runtime, ledger, and host execution stay in the main Hivra repository.

## Principles

- Modularity: plugin code is separated from core/runtime code.
- Determinism: plugin contracts are versioned and explicit.
- Downward dependencies only: plugins depend on host API contract, not vice versa.

## Layout

- `contracts/`: versioned host API contracts consumed by plugins.
- `contracts/hivra_contract_profile_v1.md`: shared contract standard (determinism, capabilities, fail-closed validation).
- `checklists/`: release and safety checklists for plugin-side contracts.
- `catalog/`: source catalog consumed by Hivra app (`plugin_catalog.json`).
- `plugins/`: plugin sources and per-plugin manifests.
- `scripts/`: local build/packaging scripts.
- `dist/plugins/`: generated plugin zip artifacts (`plugin/manifest.json` + `plugin/module.wasm`).

## Quick start

1. Install Rust with target `wasm32-unknown-unknown`.
2. Build all plugin zip packages:

```bash
./scripts/build_all_plugins.sh
```

3. Install produced zips into Hivra app from `dist/plugins/`.

## Source Catalog

`catalog/plugin_catalog.json` is the external source index for Hivra app.

It lists plugin ids, versions, and downloadable zip URLs.

Published catalog entries use schema version 2 and must pin a release tag with
an exact `sha256_hex`. Validate the catalog before release:

```bash
python3 scripts/validate_catalog.py
```

Remote catalogs may be signed with Ed25519. Keep the private key outside git:

```bash
openssl genpkey -algorithm Ed25519 -out ~/.hivra/plugin_catalog_ed25519.pem
python3 scripts/sign_catalog.py \
  --key ~/.hivra/plugin_catalog_ed25519.pem \
  --print-public-key
```

The printed raw public key hex is the value to pin in `Hivra-App`. The catalog
signature covers canonical JSON with the top-level `signatures` field removed.

Run the complete repository review:

```bash
./scripts/review_all.sh
```

macOS builds validate plugin semantics and tests but never define catalog
digests. Canonical release bytes are produced only by the pinned
`ubuntu-24.04` / Rust `1.93.0` CI builder. CI builds each candidate twice in
separate clean jobs and requires identical manifest, WASM, and ZIP bytes before
validating the signed catalog. Canonical archives use `ZIP_STORED`, fixed entry
order, timestamps, permissions, and empty extra fields.

Before a release PR can pass, download its canonical candidate artifact and
bind it mechanically while signing the catalog:

```bash
python3 scripts/sign_catalog.py \
  --key ~/.hivra/plugin_catalog_ed25519.pem \
  --dist-dir /path/to/downloaded/plugin-zips-a \
  --release-tag v0.2.8-plugins
```

The release sequence is intentionally staged: pull requests and `main` verify
source, tests, canonical archive shape, and reproducibility without requiring
the previous catalog digest. After the candidate is downloaded, bind and sign
the catalog in the release branch. Create the matching `v*` tag on that verified
commit; tagged CI requires the catalog release tag and every artifact digest to
match before publishing assets. Verify the published downloads and compatible
Capsule availability before merging the catalog into `main`, which Discovery
reads directly. Until those files exist, keep the previous catalog live.

## Included plugins

The `v0.2.12-plugins` Discovery set updates Moltbook Ambassador to 0.8.4.
Capsule Chat remains 0.2.3 and Jack Ventura remains 0.2.1. Update Capsule to
`v1.0.5-test3`, then open Plugins → Discover plugins → Install.
Jack requires its common host capabilities; `v1.0.5-test2` does not provide the
projected HTTP response and execution budget required for its full instrument
catalog. The signed catalog pins the verified canonical release packages.
Local ZIP installation remains a development path.

- `hivra.contract.capsule-chat.v1`
- `hivra.contract.moltbook-ambassador.v1` (bounded observation and deterministic
  assisted post/reply preparation; the package owns its service protocol and
  workflow, and the host executes requests within the owner's grants)
- `hivra.contract.jack-ventura.v1`: JackV `3b81753` line formation and a
  package-owned workspace through common API v2. The package owns service
  requests, account binding, entry/exit planning, response parsing and recovery;
  the host supplies scoped HTTP/signing, secret application, scheduling and an
  opaque checkpoint. No host trade controller or effect journal is required.
  Timeframes are `all` (1D/4H/1H/30M/15M/5M), `5m`, or `1m`; a managed order's
  original frame remains observed when the owner changes the selection.
  Instrument selection reads the complete public contract catalog and offers
  active USDT contracts open for API entry through the common choice action.
  The catalog is transient; only the owner-selected symbol is retained.
  First-time selection asks only for public GET access, without exchange keys
  or a scheduled workflow; connection and Start use their own explicit grant.
  Availability/rules are checked again by entry preflight. This requires a host
  advertising `response_json_paths` with a 1 MiB source bound;
  older Capsules show an update requirement instead of fetching oversized input.
  Start permits bounded autonomous cycles; Stop new entries keeps observation
  and reducing management available. Full host revocation ends new dispatch,
  without cancelling existing exchange orders or closing positions.
  The public workspace has one trading journey: connect, configure, Start/Stop
  and observe. Preparation, placement and managed cancellation are private
  cycle steps, not a second manual-entry interface. Timeframe selection uses
  the same package choice contract as instrument selection, without a new host
  operation or a stored option list.
  Pending identities survive ordinary restart; unknown delivery is reconciled
  by exact identity and never blindly resubmitted. Manual account orders are
  observed without being adopted or cancelled.
  Current stop coverage is read from the service alongside a unique matching
  managed position: active STOP_MARKET, closing direction, hedge-side or explicit
  one-way reducing/close-position semantics, optional exact position ID, remaining
  quantity and trigger source. Full/partial coverage, multiple matching stops and
  unverified observations are distinct. Coverage is transient action context,
  never an extra provider-history store; reopen, stale data, changed position or
  a failed read require fresh observation. An observed stop does not guarantee
  execution price or a maximum loss. This is observation, not automatic repair
  of missing or incorrectly sized protection.
  Entry stops remain attached to the entry request; fill-based amendment is
  outside the owner-selected scope. Actual full TP/SL closure and continuous
  VPS operation still require qualification. No live
  trade is authorized by local checks. Build with
  `./scripts/build_plugin_zip.sh jack_ventura_plugin`; package changes within
  the existing API do not require a Capsule rebuild.
