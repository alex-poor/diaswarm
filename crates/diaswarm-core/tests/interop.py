#!/usr/bin/env python3
"""Drive tools/seal.py from the Rust interop test. Not a tool; a test fixture."""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tools"))

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey, X25519PublicKey

import seal as S

RECORD = {"k": "cgm", "mgdl": 163.0, "src": "Dexcom G6", "t": 1782938503230, "trend": "FLAT"}


def identity_from(secret_hex: str) -> S.Identity:
    enc = X25519PrivateKey.from_private_bytes(bytes.fromhex(secret_hex))
    return S.Identity("interop", Ed25519PrivateKey.generate(), enc)


def main() -> int:
    cmd = sys.argv[1]

    if cmd == "seal":
        # Python seals and wraps to a reader whose public key Rust generated.
        reader_pub_hex, epoch = sys.argv[2], int(sys.argv[3])
        reader_pub = X25519PublicKey.from_public_bytes(bytes.fromhex(reader_pub_hex))
        subject = S.Identity.generate("subject")
        key = S.epoch_key()
        sealed = S.seal_epoch([RECORD], key, epoch, subject.enc_key_bytes)
        wrapped = S.wrap(key, epoch, reader_pub)
        print(json.dumps({
            "subject_pub": subject.enc_key_bytes.hex(),
            "sealed": sealed.hex(),
            "wrap": wrapped.hex(),
        }))
        return 0

    if cmd == "open":
        # Python opens what Rust sealed.
        secret_hex, epoch, subject_hex, sealed_hex, wrap_hex = sys.argv[2:7]
        reader = identity_from(secret_hex)
        key = S.unwrap(bytes.fromhex(wrap_hex), int(epoch), reader)
        records = S.open_epoch(
            bytes.fromhex(sealed_hex), key, int(epoch), bytes.fromhex(subject_hex)
        )
        print(json.dumps(records))
        return 0

    if cmd == "reader":
        # A reader keypair Python holds the secret for.
        sk = X25519PrivateKey.generate()
        from cryptography.hazmat.primitives import serialization as ser
        raw = sk.private_bytes(ser.Encoding.Raw, ser.PrivateFormat.Raw, ser.NoEncryption())
        pub = sk.public_key().public_bytes(ser.Encoding.Raw, ser.PublicFormat.Raw)
        print(json.dumps({"secret": raw.hex(), "public": pub.hex()}))
        return 0

    print(f"unknown command {cmd}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
