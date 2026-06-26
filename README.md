# hivra-plugins

External WASM plugins repository for Hivra.

This repo contains plugin packages and build/release tooling only.
Core app runtime, ledger, and host execution stay in the main Hivra repository.

## Principles

- Modularity: plugin code is separated from core/runtime code.
- Determinism: plugin contracts are versioned and explicit.
- Downward dependencies only: plugins depend on host API contract, not vice versa.

## Layout

- `contracts/`: versioned host API contracts consumed by plugins.
- `contracts/hivra_contract_profile_v1.md`: shared contract standard (determinism, capabilities, fail-closed validation).
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

Plugin archives are packaged with fixed metadata and stable entry ordering, so
identical manifest and WASM bytes produce an identical SHA-256 digest.

## Included test plugin scaffolds

- `hivra.contract.bingx-futures-trading.v1`
- `hivra.contract.capsule-chat.v1`
