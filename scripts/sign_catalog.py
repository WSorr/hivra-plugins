#!/usr/bin/env python3
import argparse
import hashlib
import json
import subprocess
import tempfile
import zipfile
from pathlib import Path
from urllib.parse import urlparse


def fail(message: str) -> None:
    raise SystemExit(f"catalog signing failed: {message}")


def run_openssl(args: list[str]) -> bytes:
    try:
        return subprocess.check_output(["openssl", *args], stderr=subprocess.PIPE)
    except FileNotFoundError:
        fail("openssl is required")
    except subprocess.CalledProcessError as error:
        detail = error.stderr.decode("utf-8", errors="replace").strip()
        fail(detail or "openssl command failed")


def canonical_json(value) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def public_key_raw_hex(private_key_path: Path) -> str:
    der = run_openssl(
        [
            "pkey",
            "-in",
            str(private_key_path),
            "-pubout",
            "-outform",
            "DER",
        ]
    )
    if len(der) < 32:
        fail("derived public key DER is too short")
    return der[-32:].hex()


def sign_payload(private_key_path: Path, payload: bytes) -> bytes:
    with tempfile.TemporaryDirectory(prefix="hivra_catalog_sign_") as tmp:
        payload_path = Path(tmp) / "payload.json"
        signature_path = Path(tmp) / "payload.sig"
        payload_path.write_bytes(payload)
        run_openssl(
            [
                "pkeyutl",
                "-sign",
                "-rawin",
                "-inkey",
                str(private_key_path),
                "-in",
                str(payload_path),
                "-out",
                str(signature_path),
            ]
        )
        return signature_path.read_bytes()


def bind_catalog_artifacts(catalog: dict, dist_dir: Path, release_tag: str) -> None:
    entries = catalog.get("entries")
    if not isinstance(entries, list) or not entries:
        fail("catalog entries must be a non-empty list")

    artifacts_by_plugin_id = {}
    for artifact_path in sorted(dist_dir.glob("*.zip")):
        try:
            with zipfile.ZipFile(artifact_path) as archive:
                manifest = json.loads(
                    archive.read("plugin/manifest.json").decode("utf-8")
                )
        except (KeyError, UnicodeDecodeError, json.JSONDecodeError, zipfile.BadZipFile) as error:
            fail(f"invalid plugin archive {artifact_path.name}: {error}")
        plugin_id = str(manifest.get("plugin_id", "")).strip()
        release_version = str(manifest.get("release_version", "")).strip()
        if not plugin_id or not release_version:
            fail(f"plugin archive missing identity: {artifact_path.name}")
        if plugin_id in artifacts_by_plugin_id:
            fail(f"duplicate plugin archive for {plugin_id}")
        artifacts_by_plugin_id[plugin_id] = (
            artifact_path,
            release_version,
            hashlib.sha256(artifact_path.read_bytes()).hexdigest(),
        )

    expected_plugin_ids = {str(entry.get("plugin_id", "")).strip() for entry in entries}
    if set(artifacts_by_plugin_id) != expected_plugin_ids:
        fail("artifact plugin ids do not exactly match catalog entries")

    for entry in entries:
        plugin_id = str(entry["plugin_id"]).strip()
        artifact_path, release_version, digest = artifacts_by_plugin_id[plugin_id]
        current_url = str(entry.get("download_url", "")).strip()
        parsed = urlparse(current_url)
        marker = "/releases/download/"
        if parsed.scheme != "https" or marker not in parsed.path:
            fail(f"catalog entry has no canonical release URL: {plugin_id}")
        repository_url = current_url.split(marker, 1)[0]
        entry["version"] = release_version
        entry["download_url"] = (
            f"{repository_url}{marker}{release_tag}/{artifact_path.name}"
        )
        entry["sha256_hex"] = digest


def main() -> None:
    root = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(
        description="Sign Hivra plugin source catalog with Ed25519.",
    )
    parser.add_argument(
        "--catalog",
        default=str(root / "catalog" / "plugin_catalog.json"),
        help="Catalog JSON path.",
    )
    parser.add_argument(
        "--key",
        required=True,
        help="Ed25519 private key PEM path. Keep it outside git.",
    )
    parser.add_argument(
        "--print-public-key",
        action="store_true",
        help="Print raw public key hex for Hivra-App pinning.",
    )
    parser.add_argument(
        "--dist-dir",
        help="Canonical CI artifact directory to bind before signing.",
    )
    parser.add_argument(
        "--release-tag",
        help="Immutable release tag used for bound artifact URLs.",
    )
    args = parser.parse_args()

    catalog_path = Path(args.catalog)
    private_key_path = Path(args.key)
    if not catalog_path.exists():
        fail(f"catalog not found: {catalog_path}")
    if not private_key_path.exists():
        fail(f"private key not found: {private_key_path}")

    catalog = json.loads(catalog_path.read_text(encoding="utf-8"))
    if not isinstance(catalog, dict):
        fail("catalog root must be a JSON object")
    if bool(args.dist_dir) != bool(args.release_tag):
        fail("--dist-dir and --release-tag must be provided together")
    if args.dist_dir:
        dist_dir = Path(args.dist_dir)
        if not dist_dir.is_dir():
            fail(f"artifact directory not found: {dist_dir}")
        bind_catalog_artifacts(catalog, dist_dir, args.release_tag)
    unsigned_catalog = dict(catalog)
    unsigned_catalog.pop("signatures", None)
    payload = canonical_json(unsigned_catalog).encode("utf-8")

    public_key_hex = public_key_raw_hex(private_key_path)
    signature = sign_payload(private_key_path, payload)
    if len(signature) != 64:
        fail(f"Ed25519 signature must be 64 bytes, got {len(signature)}")

    signed_catalog = dict(unsigned_catalog)
    signed_catalog["signatures"] = [
        {
            "algorithm": "ed25519",
            "key_id": hashlib.sha256(bytes.fromhex(public_key_hex)).hexdigest(),
            "signature_hex": signature.hex(),
        }
    ]
    catalog_path.write_text(
        json.dumps(signed_catalog, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    if args.print_public_key:
        print(public_key_hex)
    print(f"signed catalog: {catalog_path}")


if __name__ == "__main__":
    main()
