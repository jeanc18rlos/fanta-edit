# /// script
# requires-python = ">=3.10"
# dependencies = ["cryptography>=42"]
# ///
"""Fanta's release-signing key: create it once, sign each release with it.

The updater (crates/auto_update/src/verify.rs) installs an update only if this key signed
`fanta-update-v1:{asset}:{version}:{sha256}` for the downloaded installer.

  uv run script/update_signing.py keygen --private-key ~/.config/fanta/update-signing-key.pem \\
      --public-key-file crates/auto_update/release-signing-key.pub
  FANTA_UPDATE_SIGNING_KEY="$(cat key.pem)" python3 script/update_signing.py sign \\
      --asset Fanta-aarch64.dmg --version 1.1.0 --file Fanta-aarch64.dmg \\
      --public-key-file crates/auto_update/release-signing-key.pub > Fanta-aarch64.dmg.sig

`sign` reads the private key from the FANTA_UPDATE_SIGNING_KEY environment variable (PEM), never
from argv, and checks its signature against the committed public key before printing it.
"""

import argparse
import base64
import hashlib
import os
import sys
from pathlib import Path

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey, Ed25519PublicKey

DOMAIN = "fanta-update-v1"
HEADER = (
    "# Fanta's release-signing public key: raw ed25519, base64, on the first non-comment line.\n"
    "# `script/update-signing-key` writes it. A build without a key never checks for updates.\n"
)


def message(asset: str, version: str, sha256_hex: str) -> bytes:
    return f"{DOMAIN}:{asset}:{version}:{sha256_hex}".encode()


def committed_public_key(path: Path) -> Ed25519PublicKey:
    lines = [line.strip() for line in path.read_text().splitlines()]
    keys = [line for line in lines if line and not line.startswith("#")]
    if not keys:
        sys.exit(f"{path} has no release key; run script/update-signing-key first.")
    return Ed25519PublicKey.from_public_bytes(base64.b64decode(keys[0]))


def keygen(private_key: Path, public_key_file: Path) -> None:
    if private_key.exists():
        # Rotating the key would stop every installed copy from accepting updates.
        sys.exit(f"{private_key} already exists; refusing to replace the release key.")
    key = Ed25519PrivateKey.generate()
    private_key.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(private_key, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "wb") as out:
        out.write(
            key.private_bytes(
                serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()
            )
        )
    raw = key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
    public_key_file.write_text(HEADER + base64.b64encode(raw).decode() + "\n")


def sign(asset: str, version: str, file: Path, public_key_file: Path) -> None:
    pem = os.environ.get("FANTA_UPDATE_SIGNING_KEY", "").encode()
    if not pem.strip():
        sys.exit("FANTA_UPDATE_SIGNING_KEY is not set.")
    key = serialization.load_pem_private_key(pem, password=None)
    if not isinstance(key, Ed25519PrivateKey):
        sys.exit("FANTA_UPDATE_SIGNING_KEY is not an ed25519 key.")
    digest = hashlib.sha256()
    with file.open("rb") as installer:
        for chunk in iter(lambda: installer.read(1 << 20), b""):
            digest.update(chunk)
    signed = message(asset, version, digest.hexdigest())
    signature = key.sign(signed)
    try:
        committed_public_key(public_key_file).verify(signature, signed)
    except InvalidSignature:
        sys.exit("The signing key doesn't match crates/auto_update/release-signing-key.pub.")
    print(base64.b64encode(signature).decode())


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    gen = commands.add_parser("keygen")
    gen.add_argument("--private-key", type=Path, required=True)
    gen.add_argument("--public-key-file", type=Path, required=True)
    sig = commands.add_parser("sign")
    sig.add_argument("--asset", required=True)
    sig.add_argument("--version", required=True)
    sig.add_argument("--file", type=Path, required=True)
    sig.add_argument("--public-key-file", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "keygen":
        keygen(args.private_key.expanduser(), args.public_key_file)
    else:
        sign(args.asset, args.version, args.file, args.public_key_file)


if __name__ == "__main__":
    main()
