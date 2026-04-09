# Plugins

Each plugin directory is a self-contained unit:

- `Cargo.toml` + `src/` for wasm module build
- `manifest.json` for Hivra installer metadata

Packaging output format is always:

- `plugin/manifest.json`
- `plugin/module.wasm`

inside a single zip file.

