#!/usr/bin/env python3
import argparse
import copy
import hashlib
import json
import re
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path
from typing import Optional
from urllib.parse import urlparse


TRUSTED_ED25519_PUBLIC_KEYS = {
    "f4757da54ecb6f16431bcfb4fcc947633d6f68c9b4576b69ddb61fc4fe2de7a3": (
        "f36408d70b8a5b2069815a862a8f6f111e74b450226a0511058055f2969812ac"
    ),
}


def fail(message: str) -> None:
    print(f"catalog validation failed: {message}", file=sys.stderr)
    raise SystemExit(1)


def canonical_json(value) -> bytes:
    return json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
    ).encode("utf-8")


def verify_signature(catalog: dict) -> bool:
    unsigned_catalog = dict(catalog)
    signatures = unsigned_catalog.pop("signatures", [])
    payload = canonical_json(unsigned_catalog)

    for signature in signatures:
        key_id = str(signature.get("key_id", "")).strip().lower()
        public_key_hex = TRUSTED_ED25519_PUBLIC_KEYS.get(key_id)
        if public_key_hex is None:
            continue
        if hashlib.sha256(bytes.fromhex(public_key_hex)).hexdigest() != key_id:
            fail(f"trusted public key does not match key id: {key_id}")

        signature_hex = str(signature.get("signature_hex", "")).strip().lower()
        if not re.fullmatch(r"[0-9a-f]{128}", signature_hex):
            continue

        public_key_der = bytes.fromhex("302a300506032b6570032100" + public_key_hex)
        with tempfile.TemporaryDirectory(prefix="hivra_catalog_verify_") as tmp:
            tmp_path = Path(tmp)
            payload_path = tmp_path / "catalog.json"
            signature_path = tmp_path / "catalog.sig"
            public_key_path = tmp_path / "catalog-public.der"
            payload_path.write_bytes(payload)
            signature_path.write_bytes(bytes.fromhex(signature_hex))
            public_key_path.write_bytes(public_key_der)
            result = subprocess.run(
                [
                    "openssl",
                    "pkeyutl",
                    "-verify",
                    "-rawin",
                    "-pubin",
                    "-keyform",
                    "DER",
                    "-inkey",
                    str(public_key_path),
                    "-in",
                    str(payload_path),
                    "-sigfile",
                    str(signature_path),
                ],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                check=False,
            )
        if result.returncode == 0:
            return True
    return False


def validate_catalog(catalog: dict, expected_release_tag: Optional[str] = None) -> None:
    if catalog.get("schema") != "hivra.plugin.catalog":
        fail("unsupported schema")
    if catalog.get("version") != 2:
        fail("published catalog must use version 2")

    entries = catalog.get("entries")
    if not isinstance(entries, list) or not entries:
        fail("entries must be a non-empty list")

    signatures = catalog.get("signatures", [])
    if not isinstance(signatures, list) or not signatures:
        fail("published catalog must include at least one signature")
    for signature in signatures:
        if not isinstance(signature, dict):
            fail("signature entries must be objects")
        if signature.get("algorithm") != "ed25519":
            fail("signature algorithm must be ed25519")
        key_id = str(signature.get("key_id", "")).strip().lower()
        if not re.fullmatch(r"[0-9a-f]{64}", key_id):
            fail("signature key_id must be sha256 hex")
        signature_hex = str(signature.get("signature_hex", "")).strip().lower()
        if not re.fullmatch(r"[0-9a-f]{128}", signature_hex):
            fail("signature_hex must be 64-byte hex")

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
        if expected_release_tag is not None:
            release_tag = parsed.path.split("/releases/download/", 1)[1].split("/", 1)[0]
            if release_tag != expected_release_tag:
                fail(
                    f"{entry_id}: catalog release tag {release_tag!r} does not "
                    f"match requested release tag {expected_release_tag!r}"
                )

    if not verify_signature(catalog):
        fail("catalog has no valid signature from a trusted signer")


def run_negative_self_tests(catalog: dict) -> None:
    mutated_entry = copy.deepcopy(catalog)
    mutated_entry["entries"][0]["display_name"] += " mutation"
    if verify_signature(mutated_entry):
        fail("entry mutation did not invalidate the catalog signature")

    mutated_signature = copy.deepcopy(catalog)
    signature_hex = mutated_signature["signatures"][0]["signature_hex"]
    replacement = "0" if signature_hex[0] != "0" else "1"
    mutated_signature["signatures"][0]["signature_hex"] = (
        replacement + signature_hex[1:]
    )
    if verify_signature(mutated_signature):
        fail("signature mutation was accepted")


def _read_canonical_artifact(artifact_path: Path) -> tuple[dict, str, str, str]:
    try:
        with zipfile.ZipFile(artifact_path) as archive:
            archive_entries = archive.infolist()
            if [item.filename for item in archive_entries] != [
                "plugin/manifest.json",
                "plugin/module.wasm",
            ]:
                fail(f"{artifact_path.name}: archive entry order is not canonical")
            for item in archive_entries:
                if (
                    item.date_time != (1980, 1, 1, 0, 0, 0)
                    or item.compress_type != zipfile.ZIP_STORED
                    or item.create_system != 3
                    or item.external_attr != 0o100644 << 16
                    or item.extra
                    or item.comment
                ):
                    fail(f"{artifact_path.name}: archive metadata is not canonical")
            manifest_bytes = archive.read("plugin/manifest.json")
            manifest = json.loads(manifest_bytes)
            module_digest = hashlib.sha256(archive.read("plugin/module.wasm")).hexdigest()
    except (KeyError, UnicodeDecodeError, json.JSONDecodeError, zipfile.BadZipFile) as error:
        fail(f"{artifact_path.name}: invalid plugin archive: {error}")
    if not isinstance(manifest, dict):
        fail(f"{artifact_path.name}: manifest must be an object")
    plugin_id = str(manifest.get("plugin_id", "")).strip()
    version = str(manifest.get("release_version", "")).strip()
    if not plugin_id or not version:
        fail(f"{artifact_path.name}: manifest identity is incomplete")
    return (
        manifest,
        hashlib.sha256(artifact_path.read_bytes()).hexdigest(),
        hashlib.sha256(manifest_bytes).hexdigest(),
        module_digest,
    )


def validate_dist(catalog: dict, dist_dir: Path, check_digests: bool = True) -> None:
    if not check_digests:
        artifacts = sorted(dist_dir.glob("*.zip"))
        if not artifacts:
            fail(f"no plugin artifacts found in {dist_dir}")
        seen_plugin_ids: set[str] = set()
        for artifact_path in artifacts:
            manifest, actual_digest, manifest_digest, module_digest = _read_canonical_artifact(
                artifact_path
            )
            plugin_id = str(manifest["plugin_id"]).strip()
            if plugin_id in seen_plugin_ids:
                fail(f"duplicate plugin artifact for {plugin_id}")
            seen_plugin_ids.add(plugin_id)
            print(
                "canonical artifact shape: "
                f"file={artifact_path.name} plugin_id={plugin_id} "
                f"version={manifest['release_version']} zip={actual_digest} "
                f"manifest={manifest_digest} wasm={module_digest}"
            )
        return

    mismatches = []
    for entry in catalog["entries"]:
        artifact_name = Path(urlparse(entry["download_url"]).path).name
        artifact_path = dist_dir / artifact_name
        if not artifact_path.is_file():
            fail(f"{entry['id']}: missing built artifact {artifact_name}")
        manifest, actual_digest, manifest_digest, wasm_digest = _read_canonical_artifact(
            artifact_path
        )
        if manifest.get("plugin_id") != entry.get("plugin_id"):
            fail(f"{entry['id']}: artifact plugin_id does not match catalog")
        if manifest.get("release_version") != entry.get("version"):
            fail(f"{entry['id']}: artifact release_version does not match catalog")
        print(
            "catalog artifact hashes: "
            f"id={entry['id']} zip={actual_digest} "
            f"manifest={manifest_digest} wasm={wasm_digest}"
        )
        if actual_digest != entry["sha256_hex"]:
            mismatches.append(
                f"{entry['id']}: built artifact digest does not match catalog; "
                f"expected_zip={entry['sha256_hex']} actual_zip={actual_digest} "
                f"manifest={manifest_digest} wasm={wasm_digest}"
            )
    if mismatches:
        fail("\n".join(mismatches))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--dist-dir", type=Path)
    parser.add_argument(
        "--allow-unbound-artifacts",
        action="store_true",
        help="Validate canonical package shape without requiring current catalog digests.",
    )
    parser.add_argument(
        "--expected-release-tag",
        help="Require every catalog URL to use this release tag.",
    )
    args = parser.parse_args()

    root = Path(__file__).resolve().parent.parent
    catalog_path = root / "catalog" / "plugin_catalog.json"
    catalog = json.loads(catalog_path.read_text(encoding="utf-8"))
    if not isinstance(catalog, dict):
        fail("catalog root must be a JSON object")

    if args.allow_unbound_artifacts and args.expected_release_tag:
        fail("--allow-unbound-artifacts cannot be combined with --expected-release-tag")
    validate_catalog(catalog, args.expected_release_tag)
    if args.self_test:
        run_negative_self_tests(catalog)
        print("catalog signature negative self-tests passed")
    if args.dist_dir is not None:
        validate_dist(
            catalog,
            args.dist_dir,
            check_digests=not args.allow_unbound_artifacts,
        )
        print("catalog artifact digest validation passed")
    print(f"catalog validation passed: {len(catalog['entries'])} entries")


if __name__ == "__main__":
    main()
