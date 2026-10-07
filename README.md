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

## Included test plugin scaffolds

- `hivra.contract.capsule-chat.v1`
- `hivra.contract.moltbook-ambassador.v1` (bounded observation and deterministic
  assisted post/reply preparation; network effects remain host-owned)
- `hivra.contract.jack-ventura.v1`: JackV `3b81753` line formation and
  package-owned workspace on
  `1D/4H/1H/30M/15M/5M`. Settings, swings, ATR, first-known times and touched
  levels live in WASM state. It requests public candles and normalized
  read-only account evidence through `plugin_workspace_v1`, and computes
  quantity and stop previews from margin and existing directional leverage.
  Keys remain in the host; advanced settings and calculation evidence are
  optional. It can prepare one immutable entry for explicit host confirmation
  and interpret exact provider-order evidence after restart. The host reads
  current selected-instrument open orders; WASM presents their transient
  list without adopting manual orders or storing another order history.
  The host owns signing and the existing durable effect journal; uncertain outcomes do not
  authorize duplicate entries. Attached stop requests do not prove protection.
  Autonomous trading, fill-based protection/profit exits and VPS operation
  remain incomplete. Build with `./scripts/build_plugin_zip.sh jack_ventura_plugin`
  and install the local ZIP in a host supporting that workspace contract.
