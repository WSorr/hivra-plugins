#!/usr/bin/env python3
import sys
import zipfile
from pathlib import Path


if len(sys.argv) != 4:
    raise SystemExit("usage: package_plugin.py <manifest> <wasm> <output.zip>")

manifest_path = Path(sys.argv[1])
wasm_path = Path(sys.argv[2])
output_path = Path(sys.argv[3])
output_path.parent.mkdir(parents=True, exist_ok=True)

entries = (
    ("plugin/manifest.json", manifest_path.read_bytes()),
    ("plugin/module.wasm", wasm_path.read_bytes()),
)

with zipfile.ZipFile(
    output_path,
    mode="w",
    compression=zipfile.ZIP_DEFLATED,
    compresslevel=9,
) as archive:
    for name, payload in entries:
        info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
        info.compress_type = zipfile.ZIP_DEFLATED
        info.create_system = 3
        info.external_attr = 0o100644 << 16
        archive.writestr(info, payload)
