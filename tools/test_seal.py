#!/usr/bin/env python3
"""
Tests for seal.py. No dependencies beyond `cryptography`, no network, no data:

    tools/test_seal.py

WHAT THESE ARE FOR. One property decides whether this design is honest:

    after revocation the reader decrypts nothing new, and everything they
    already held still opens

Both halves are tested, and so are the ways the mechanism could be quietly
wrong while still looking right — a reader opening another reader's wrap, a
sealed epoch being passed off as a different epoch, a grant record that has been
edited. A revocation that works only when nobody attacks it is not a revocation.
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

import seal  # noqa: E402
from canon import EPOCH_MS  # noqa: E402

FAILURES: list[str] = []


def check(name: str, condition: bool, detail: str = "") -> None:
    if condition:
        print(f"  ok    {name}")
    else:
        print(f"  FAIL  {name}   {detail}")
        FAILURES.append(name)


def raises(fn) -> bool:
    try:
        fn()
        return False
    except Exception:
        return True


def history(days: int, start_epoch: int = 20_000) -> dict[int, list[dict]]:
    """A synthetic history: one CGM reading per epoch, values that identify it."""
    return {
        start_epoch + d: [{"t": (start_epoch + d) * EPOCH_MS + 3_600_000,
                           "k": "cgm", "mgdl": 100.0 + d}]
        for d in range(days)
    }


def live_run(days: int, revoke_after: int):
    """Seal day by day, with a revocation landing mid-history.

    Sealing everything up front would make "nothing new" true by construction —
    there would be nothing new to gain. The revocation has to land with history
    still to come or the test proves nothing.
    """
    subject = seal.Identity.generate("subject")
    a = seal.Identity.generate("a")
    b = seal.Identity.generate("b")
    epochs = history(days)
    order = sorted(epochs)

    v = seal.Vault(subject)
    v.grant(a, "follow", order[0])
    v.grant(b, "follow", order[0])

    held_at_stop = None
    for i, e in enumerate(order):
        if i == revoke_after:
            v.stop(a, "follow", e)
            held_at_stop = set(v.read(a))
        v.seal_one(e, epochs[e])
        for who in (a, b):
            v.publish_wraps(who, "follow")
    return subject, a, b, v, order, held_at_stop


def main() -> int:
    days, cut = 10, 6
    subject, a, b, v, order, held_at_stop = live_run(days, cut)
    after = set(v.read(a))

    # --- the property -------------------------------------------------------
    check("a revoked reader gains nothing new", not (after - held_at_stop),
          f"gained {sorted(after - held_at_stop)}")
    check("a revoked reader keeps everything they held", held_at_stop <= after,
          f"lost {sorted(held_at_stop - after)}")
    check("revocation actually bites", len(after) == cut,
          f"holds {len(after)} of {len(order)}, expected {cut}")
    check("revocation is per-recipient", len(v.read(b)) == days,
          f"b holds {len(v.read(b))} of {days}")

    # --- what a revoked reader holds is really readable, not just present ---
    opened = v.read(a)
    check("the epochs a revoked reader keeps still decrypt to the right records",
          all(opened[e][0]["mgdl"] == 100.0 + i for i, e in enumerate(sorted(opened))),
          f"{[r[0]['mgdl'] for r in opened.values()]}")

    # --- a holder of ciphertext learns nothing ------------------------------
    stranger = seal.Identity.generate("stranger")
    check("a peer holding the sealed bytes cannot read them",
          v.read(stranger) == {}, "a stranger opened something")
    check("a stranger cannot unwrap a wrap addressed to someone else",
          raises(lambda: seal.unwrap(
              v.wraps[(order[0], a.enc_key_bytes.hex())], order[0], stranger)))

    # --- confusion attacks --------------------------------------------------
    key = seal.unwrap(v.wraps[(order[0], a.enc_key_bytes.hex())], order[0], a)
    check("a sealed epoch cannot be opened as a different epoch",
          raises(lambda: seal.open_epoch(
              v.sealed[order[0]], key, order[1], subject.enc_key_bytes)))
    check("a sealed epoch cannot be attributed to a different subject",
          raises(lambda: seal.open_epoch(
              v.sealed[order[0]], key, order[0], stranger.enc_key_bytes)))
    check("a wrap for one epoch does not unwrap another",
          raises(lambda: seal.unwrap(
              v.wraps[(order[0], a.enc_key_bytes.hex())], order[1], a)))
    check("one epoch's key does not open another epoch",
          raises(lambda: seal.open_epoch(
              v.sealed[order[1]], key, order[1], subject.enc_key_bytes)))

    # --- grant records ------------------------------------------------------
    g = v.grants[0]
    check("a grant record verifies against the subject",
          seal.verify_grant(g, subject.verify_key))
    check("an edited grant record does not verify",
          not seal.verify_grant({**g, "purpose": "cohort"}, subject.verify_key))
    check("a grant record does not verify against another key",
          not seal.verify_grant(g, stranger.verify_key))
    check("the grant log records the withdrawal, not just the grant",
          any(x["act"] == "stop" for x in v.grants))

    # --- entitlement is a function of the grant log -------------------------
    e0 = order[0]
    log = [seal.grant_record(subject, a, "p", "grant", e0),
           seal.grant_record(subject, a, "p", "stop", e0 + 2),
           seal.grant_record(subject, a, "p", "grant", e0 + 5)]
    got = seal.live_epochs(log, a, "p", list(range(e0, e0 + 7)))
    check("grant, stop and re-grant compose in epoch order",
          got == {e0, e0 + 1, e0 + 5, e0 + 6}, f"{sorted(x - e0 for x in got)}")
    check("a grant for one purpose does not entitle another",
          seal.live_epochs(log, a, "other", list(range(e0, e0 + 7))) == set())

    # --- keys are independent ----------------------------------------------
    keys = {seal.epoch_key() for _ in range(64)}
    check("epoch keys are independent", len(keys) == 64)

    # --- the wrap is the size the design was costed on ----------------------
    w = len(next(iter(v.wraps.values())))
    check("a wrap is about 100 bytes, as §7.2 assumed", 80 <= w <= 120, f"{w} bytes")

    print()
    if FAILURES:
        print(f"  {len(FAILURES)} FAILED: {', '.join(FAILURES)}")
        return 1
    print(f"  all checks pass over {days} epochs")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
