#!/usr/bin/env python3
"""
Seal a canonical record stream into epochs, and wrap each epoch key to the
people who were granted it.

This is stage 10.2 of docs/feasibility.md, built the way stage 10.1 was: as the
framework-neutral floor, so the property can be demonstrated on real data today
rather than on an API nobody here has used yet.

THE ONE PROPERTY THIS EXISTS TO DEMONSTRATE.

  After revocation the reader decrypts NOTHING NEW, and everything they
  ALREADY HELD still opens.

  That is the honest promise in feasibility.md §11 — "nothing new will be sent
  after you stop it" — as a mechanism rather than a sentence. Both halves
  matter. A design where the reader keeps reading is a broken revocation; a
  design where their existing copy goes dark is claiming a recall it cannot
  perform, and §11 says never to claim that.

WHY PER-RECIPIENT WRAPPING AND NOT A CGKA.

  §7.3 recommends it and the reasoning holds: at five readers the O(n) cost is
  meaningless, and the mechanism is boring, obvious and auditable. p2panda's
  data mode gives the same guarantee with a group key, and the shipping version
  should use it — this is the reference the shipping version has to match, not
  a competitor to it. Anything here that p2panda does differently is a question
  about p2panda, and this file is how you notice.

WHAT THIS IS NOT.

  * NOT A SWARM. It writes files. Where those bytes go, how they replicate and
    who holds them is stages beyond this one; nothing here has an opinion.
  * NOT REVIEWED CRYPTOGRAPHY. Standard primitives, composed by hand:
    X25519 + HKDF-SHA256 + ChaCha20-Poly1305, Ed25519 for grants. Stage 10.6
    is an external review gate and this is exactly the kind of thing it exists
    to catch.
  * NOT FORWARD-SECRET WITHIN AN EPOCH. A compromised reader device exposes
    every epoch that reader was ever wrapped, permanently. feasibility.md §7.4
    names this as a permanent property of the architecture, "strictly worse
    than the fetch model, and the sharpest thing to say out loud".
  * NOT A DELETION MECHANISM. Sealed epochs, once published, are published.

Usage:
    tools/seal.py stream.ndjson --demo
    tools/seal.py stream.ndjson --out vault/ --grant partner --grant clinic
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from collections import defaultdict
from pathlib import Path

from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import (
    Ed25519PrivateKey,
    Ed25519PublicKey,
)
from cryptography.hazmat.primitives.asymmetric.x25519 import (
    X25519PrivateKey,
    X25519PublicKey,
)
from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305
from cryptography.hazmat.primitives.kdf.hkdf import HKDF

sys.path.insert(0, str(Path(__file__).parent))
from canon import EPOCH_MS, epoch_of  # noqa: E402

# Bumping this changes every derived key, so it is a wire-compatibility break in
# the same way a record schema change is. It exists so that a future scheme can
# be told apart from this one rather than silently producing garbage.
WRAP_INFO = b"diaswarm-wrap-v1"
SEAL_INFO = b"diaswarm-seal-v1"

KEY_BYTES = 32
NONCE_BYTES = 12


# --------------------------------------------------------------------------
# Identity
# --------------------------------------------------------------------------

class Identity:
    """One party: a signing key for grants, an encryption key for wraps.

    Two keys rather than one because they are used by different parties for
    different things — the subject signs grants that everyone verifies, and a
    reader decrypts wraps that only they can open. Deriving one from the other
    is possible and is the kind of cleverness a review exists to object to.
    """

    def __init__(self, name: str, sign: Ed25519PrivateKey, enc: X25519PrivateKey):
        self.name = name
        self._sign = sign
        self._enc = enc

    @classmethod
    def generate(cls, name: str) -> Identity:
        return cls(name, Ed25519PrivateKey.generate(), X25519PrivateKey.generate())

    @property
    def verify_key(self) -> Ed25519PublicKey:
        return self._sign.public_key()

    @property
    def enc_key(self) -> X25519PublicKey:
        return self._enc.public_key()

    @property
    def enc_key_bytes(self) -> bytes:
        return self.enc_key.public_bytes(
            serialization.Encoding.Raw, serialization.PublicFormat.Raw
        )

    def sign(self, payload: bytes) -> bytes:
        return self._sign.sign(payload)

    def exchange(self, peer_pub: X25519PublicKey) -> bytes:
        return self._enc.exchange(peer_pub)


# --------------------------------------------------------------------------
# Epoch keys: sealing the data
# --------------------------------------------------------------------------

def _derive(shared: bytes, info: bytes, context: bytes) -> bytes:
    return HKDF(
        algorithm=hashes.SHA256(), length=KEY_BYTES, salt=None, info=info + context
    ).derive(shared)


def epoch_key() -> bytes:
    """One content key per epoch. Independent of every other epoch's key.

    Independence is the whole construction. Derived keys — a chain, a ratchet, a
    KDF from a master — would mean handing a reader epoch 40 tells them
    something about epoch 41, and revocation would stop meaning what it says.
    """
    return os.urandom(KEY_BYTES)


def seal_epoch(records: list[dict], key: bytes, epoch: int, subject: bytes) -> bytes:
    """Seal one epoch's records under its content key.

    The epoch number and the subject's identity are authenticated but not
    encrypted: a holder must be able to route and replicate a sealed epoch
    without being able to read it, and must not be able to pass epoch 12 off as
    epoch 13 to a reader who was only granted 13.
    """
    from canon import _canon_json  # local import: canon owns the encoding

    plaintext = b"".join(_canon_json(r).encode() + b"\n" for r in records)
    nonce = os.urandom(NONCE_BYTES)
    aad = subject + epoch.to_bytes(8, "big")
    return nonce + ChaCha20Poly1305(key).encrypt(nonce, plaintext, aad)


def open_epoch(sealed: bytes, key: bytes, epoch: int, subject: bytes) -> list[dict]:
    """Open a sealed epoch. Raises if the key is wrong or the epoch was moved."""
    nonce, ct = sealed[:NONCE_BYTES], sealed[NONCE_BYTES:]
    aad = subject + epoch.to_bytes(8, "big")
    plaintext = ChaCha20Poly1305(key).decrypt(nonce, ct, aad)
    return [json.loads(line) for line in plaintext.splitlines() if line]


# --------------------------------------------------------------------------
# Wraps: who can open which epoch
# --------------------------------------------------------------------------

def wrap(key: bytes, epoch: int, reader_pub: X25519PublicKey) -> bytes:
    """Wrap an epoch key to one reader. About 92 bytes.

    Ephemeral-static X25519: a fresh ephemeral key per wrap, so the subject's
    long-term key is never the only thing between a reader and every epoch.
    """
    eph = X25519PrivateKey.generate()
    reader_raw = reader_pub.public_bytes(
        serialization.Encoding.Raw, serialization.PublicFormat.Raw
    )
    shared = eph.exchange(reader_pub)
    wrapping = _derive(shared, WRAP_INFO, epoch.to_bytes(8, "big") + reader_raw)
    nonce = os.urandom(NONCE_BYTES)
    ct = ChaCha20Poly1305(wrapping).encrypt(nonce, key, reader_raw)
    eph_raw = eph.public_key().public_bytes(
        serialization.Encoding.Raw, serialization.PublicFormat.Raw
    )
    return eph_raw + nonce + ct


def unwrap(wrapped: bytes, epoch: int, reader: Identity) -> bytes:
    """Recover an epoch key from a wrap addressed to this reader."""
    eph_raw, nonce, ct = wrapped[:32], wrapped[32:32 + NONCE_BYTES], wrapped[32 + NONCE_BYTES:]
    shared = reader.exchange(X25519PublicKey.from_public_bytes(eph_raw))
    reader_raw = reader.enc_key_bytes
    wrapping = _derive(shared, WRAP_INFO, epoch.to_bytes(8, "big") + reader_raw)
    return ChaCha20Poly1305(wrapping).decrypt(nonce, ct, reader_raw)


# --------------------------------------------------------------------------
# Grants: the public, signed, tamper-evident record
# --------------------------------------------------------------------------

def grant_record(subject: Identity, reader: Identity, purpose: str,
                 act: str, epoch: int) -> dict:
    """A signed statement that a grant started or stopped.

    This is the whole of what feasibility.md §11 promises in place of a read
    log: every grant and every withdrawal, signed, and not rewritable by the
    reader. It is deliberately NOT a record of anyone reading — nothing here can
    produce one, and §11 forbids implying otherwise.

    NOTE, and it is §12.0: this record is public and it names the reader. The
    data is encrypted; the social graph is not. That is the sharpest unsolved
    problem in the design and this function is where it becomes concrete.
    """
    body = {
        "act": act,                       # "grant" | "stop"
        "epoch": epoch,                   # from this epoch, inclusive
        "purpose": purpose,
        "reader": reader.enc_key_bytes.hex(),
        "subject": subject.enc_key_bytes.hex(),
    }
    payload = json.dumps(body, separators=(",", ":"), sort_keys=True).encode()
    return {**body, "sig": subject.sign(payload).hex()}


def verify_grant(record: dict, subject_verify: Ed25519PublicKey) -> bool:
    body = {k: v for k, v in record.items() if k != "sig"}
    payload = json.dumps(body, separators=(",", ":"), sort_keys=True).encode()
    try:
        subject_verify.verify(bytes.fromhex(record["sig"]), payload)
        return True
    except Exception:
        return False


def live_epochs(grants: list[dict], reader: Identity, purpose: str,
                epochs: list[int]) -> set[int]:
    """Which epochs a reader is currently entitled to, from the grant records.

    Granting starts the wrapping and revoking stops it, so entitlement is a
    property of the grant log rather than of anything a server decides. An
    epoch is live if the most recent statement at or before it was a grant.
    """
    mine = sorted(
        (g for g in grants
         if g["reader"] == reader.enc_key_bytes.hex() and g["purpose"] == purpose),
        key=lambda g: g["epoch"],
    )
    live, state = set(), None
    for e in sorted(epochs):
        for g in mine:
            if g["epoch"] <= e:
                state = g["act"]
        if state == "grant":
            live.add(e)
    return live


# --------------------------------------------------------------------------
# The vault: one subject's sealed history plus its wraps and grants
# --------------------------------------------------------------------------

def by_epoch(records: list[dict]) -> dict[int, list[dict]]:
    """Group records into UTC-day epochs. The header (t=0) is not an event."""
    out: dict[int, list[dict]] = defaultdict(list)
    for r in records:
        if r.get("k") == "meta":
            continue
        out[epoch_of(r["t"])].append(r)
    return dict(out)


class Vault:
    """Everything a subject publishes: sealed epochs, wraps, grants."""

    def __init__(self, subject: Identity):
        self.subject = subject
        self.sealed: dict[int, bytes] = {}
        self.wraps: dict[tuple[int, str], bytes] = {}
        self.grants: list[dict] = []
        self._keys: dict[int, bytes] = {}

    def seal_one(self, epoch: int, records: list[dict]) -> None:
        """Seal one epoch. This is what a phone does once a day."""
        key = epoch_key()
        self._keys[epoch] = key
        self.sealed[epoch] = seal_epoch(records, key, epoch, self.subject.enc_key_bytes)

    def seal(self, epochs: dict[int, list[dict]]) -> None:
        for epoch, records in sorted(epochs.items()):
            self.seal_one(epoch, records)

    def grant(self, reader: Identity, purpose: str, epoch: int) -> None:
        self.grants.append(grant_record(self.subject, reader, purpose, "grant", epoch))

    def stop(self, reader: Identity, purpose: str, epoch: int) -> None:
        self.grants.append(grant_record(self.subject, reader, purpose, "stop", epoch))

    def publish_wraps(self, reader: Identity, purpose: str) -> int:
        """Wrap every epoch this reader is currently entitled to. Returns the count.

        THIS IS REVOCATION. There is no delete step and no message to anyone: a
        stopped reader is simply one this loop no longer wraps for. What they
        already hold is untouched, which is the honest half of the promise.
        """
        entitled = live_epochs(self.grants, reader, purpose, list(self.sealed))
        published = 0
        for epoch in sorted(entitled):
            slot = (epoch, reader.enc_key_bytes.hex())
            if slot in self.wraps:
                continue          # already published; a wrap is not re-issued
            self.wraps[slot] = wrap(self._keys[epoch], epoch, reader.enc_key)
            published += 1
        return published

    def read(self, reader: Identity) -> dict[int, list[dict]]:
        """Everything this reader can actually open, from wraps alone.

        Deliberately does not consult the grant log. A grant record is a public
        statement about intent; what a reader can read is decided by which keys
        they hold, and the two are only equal if the mechanism is honest.
        """
        out = {}
        for epoch in sorted(self.sealed):
            w = self.wraps.get((epoch, reader.enc_key_bytes.hex()))
            if w is None:
                continue
            key = unwrap(w, epoch, reader)
            out[epoch] = open_epoch(
                self.sealed[epoch], key, epoch, self.subject.enc_key_bytes
            )
        return out

    def bytes_on_the_wire(self) -> tuple[int, int]:
        return (sum(len(b) for b in self.sealed.values()),
                sum(len(b) for b in self.wraps.values()))


# --------------------------------------------------------------------------
# The demonstration
# --------------------------------------------------------------------------

def demo(records: list[dict]) -> int:
    """Seal a real stream, grant three readers, revoke one, and report.

    The output is the point. Anyone can assert that revocation works; this
    prints what each reader can open before and after, from a real history.
    """
    subject = Identity.generate("subject")
    partner = Identity.generate("partner")
    clinic = Identity.generate("clinic")
    cohort = Identity.generate("cohort")

    epochs = by_epoch(records)
    order = sorted(epochs)
    first = order[0]
    revoke_at = order[int(len(order) * 2 / 3)]
    window = (order[len(order) // 4], order[len(order) // 4 + 6])
    readers = ((partner, "follow"), (clinic, "clinician"), (cohort, "cohort"))

    vault = Vault(subject)

    # Grants, as a subject would make them, before any of this history exists.
    vault.grant(partner, "follow", first)
    vault.grant(clinic, "clinician", first)
    vault.grant(cohort, "cohort", window[0])
    vault.stop(cohort, "cohort", window[1] + 1)   # a fixed window, agreed up front

    # SEALED A DAY AT A TIME, because that is what a phone does — and because
    # sealing everything first would make "nothing new after the stop" true by
    # construction rather than by mechanism. The revocation has to land with
    # history still to come, or it demonstrates nothing.
    held_before: set[int] = set()
    for epoch in order:
        if epoch == revoke_at:
            vault.stop(partner, "follow", epoch)
            held_before = set(vault.read(partner))
        vault.seal_one(epoch, epochs[epoch])
        for who, purpose in readers:
            vault.publish_wraps(who, purpose)

    print(f"\n  sealed  {len(epochs)} epochs, "
          f"{sum(len(v) for v in epochs.values()):,} records, one day at a time")
    print(f"  partner revoked at epoch {revoke_at}, with "
          f"{sum(1 for e in order if e >= revoke_at)} of {len(order)} epochs still to come")

    held_after = set(vault.read(partner))
    kept = held_before & held_after
    gained = held_after - held_before

    print(f"\n  what each reader can open, at the end")
    for who, purpose in readers:
        opened = vault.read(who)
        print(f"    {who.name:<9} {len(opened):>3} of {len(order)} epochs  {purpose}")

    print(f"\n  the property")
    print(f"    nothing new          {len(gained)} epochs gained after the stop"
          f"   {'PASS' if not gained else 'FAIL'}")
    print(f"    nothing recalled     {len(kept)}/{len(held_before)} previously held epochs"
          f" still open   {'PASS' if kept == held_before else 'FAIL'}")
    print(f"    per-recipient        clinic unaffected: {len(vault.read(clinic))} of "
          f"{len(order)} epochs   {'PASS' if len(vault.read(clinic)) == len(order) else 'FAIL'}")
    print(f"    revocation bites     partner holds {len(held_after)} of {len(order)}, "
          f"blind to {len(order) - len(held_after)}   "
          f"{'PASS' if len(held_after) < len(order) else 'FAIL'}")
    print(f"    time-scoped          cohort holds {len(vault.read(cohort))} epochs of "
          f"{len(order)}")

    data, keys = vault.bytes_on_the_wire()
    one_wrap = len(next(iter(vault.wraps.values())))
    # Scaling the mix above would understate it: one of those three readers was
    # revoked a third of the way in and another held a seven-epoch window. The
    # number §7.2 estimated is five readers granted throughout, so quote that.
    full_year = 5 * 365 * one_wrap
    print(f"\n  on the wire")
    print(f"    sealed data  {data / 1e6:>8.2f} MB")
    print(f"    key records  {keys / 1024:>8.1f} KB   {len(vault.wraps)} wraps of "
          f"{one_wrap} bytes, for this mix of three readers")
    print(f"    five readers {full_year / 1024:>8.0f} KB/year   granted throughout, "
          f"against §7.2's estimate of 180")

    ok = not gained and kept == held_before and len(held_after) < len(order)
    print(f"\n  {'the promise holds' if ok else 'THE PROMISE DOES NOT HOLD'}\n")
    return 0 if ok else 1


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[1])
    ap.add_argument("stream", type=Path, help="canonical NDJSON from canon.py")
    ap.add_argument("--demo", action="store_true", help="grant, revoke, and report")
    args = ap.parse_args()

    if not args.stream.exists():
        print(f"no such stream: {args.stream}", file=sys.stderr)
        return 1

    records = [json.loads(l) for l in args.stream.read_text().splitlines() if l]
    header = next((r for r in records if r.get("k") == "meta"), None)
    if header is None:
        print("  WARNING: no header — cannot tell which spec version this is,\n"
              "           or whether its glucose values are normalised.", file=sys.stderr)
    elif header.get("epoch") != "utc-day":
        print(f"  refusing: stream declares epoch basis {header.get('epoch')!r}, "
              f"this seals utc-day", file=sys.stderr)
        return 1

    if args.demo:
        return demo(records)

    print("nothing to do — pass --demo", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
