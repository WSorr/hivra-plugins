#!/usr/bin/env python3
import json
import re
import sys
from pathlib import Path
from urllib.parse import urlparse


def fail(message: str) -> None:
    print(f"catalog validation failed: {message}", file=sys.stderr)
    raise SystemExit(1)


root = Path(__file__).resolve().parent.parent
catalog_path = root / "catalog" / "plugin_catalog.json"
catalog = json.loads(catalog_path.read_text(encoding="utf-8"))

if catalog.get("schema") != "hivra.plugin.catalog":
    fail("unsupported schema")
if catalog.get("version") != 2:
    fail("published catalog must use version 2")

entries = catalog.get("entries")
if not isinstance(entries, list) or not entries:
    fail("entries must be a non-empty list")

seen_ids: set[str] = set()
for entry in entries:
    entry_id = str(entry.get("id", "")).strip()
    if not entry_id or entry_id in seen_ids:
        fail(f"invalid or duplicate entry id: {entry_id!r}")
    seen_ids.add(entry_id)

    digest = str(entry.get("sha256_hex", "")).strip().lower()
    if not re.fullmatch(r"[0-9a-f]{64}", digest):
        fail(f"{entry_id}: sha256_hex is required")

    download_url = str(entry.get("download_url", "")).strip()
    parsed = urlparse(download_url)
    if parsed.scheme != "https":
        fail(f"{entry_id}: download_url must use https")
    if "/releases/latest/" in parsed.path:
        fail(f"{entry_id}: latest release URLs are forbidden")
    if not re.search(r"/releases/download/[^/]+/[^/]+$", parsed.path):
        fail(f"{entry_id}: download_url must pin a release tag")

print(f"catalog validation passed: {len(entries)} entries")
