#!/usr/bin/env python3
"""Write .sig sidecars for release binaries (Ed25519 over file bytes)."""

from __future__ import annotations

import binascii
import os
import sys

from cryptography.hazmat.primitives.asymmetric import ed25519


def main() -> int:
    key_hex = os.environ.get("SIGNING_KEY", "").strip()
    if not key_hex:
        print("SIGNING_KEY not set; skipping Ed25519 signatures.", file=sys.stderr)
        return 0

    if len(sys.argv) < 2:
        print("usage: sign-ed25519.py <exe> [exe ...]", file=sys.stderr)
        return 1

    priv = ed25519.Ed25519PrivateKey.from_private_bytes(binascii.unhexlify(key_hex))

    for path in sys.argv[1:]:
        with open(path, "rb") as f:
            data = f.read()
        sig = priv.sign(data)
        sig_path = f"{path}.sig"
        with open(sig_path, "wb") as f:
            f.write(sig)
        print(f"Wrote {sig_path}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
