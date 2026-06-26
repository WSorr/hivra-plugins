#!/usr/bin/env python3
import argparse
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path


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
