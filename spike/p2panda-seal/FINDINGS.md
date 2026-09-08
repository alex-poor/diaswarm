# p2panda-encryption 0.7.1, measured

**Run:** `cargo run --manifest-path spike/p2panda-seal/Cargo.toml`

The question from feasibility.md §10.2 was whether the library that ships gives
the property [`tools/seal.py`](../../tools/seal.py) demonstrates, and at what
granularity. Three answers, and one of them changes a claim in the design.

---

## 1. The property holds

```
partner   holds  7 secrets, opens epochs [0, 1, 2, 3, 4, 5]
clinic    holds 11 secrets, opens epochs [0..9]

  nothing new after removal              PASS
  everything already held still opens    PASS   6 of 6 pre-removal epochs
  revocation is per-recipient            PASS   clinic opens 10 of 10
```

A member removed at epoch 6, with four epochs still to come, opens none of them
and keeps all six it held. **Same guarantee as the reference, from the library.**

Measured against the keys a reader still holds and the bytes it still holds —
not by replaying messages through `receive()`, which refuses anything it has
already seen and would have reported an empty result for *everyone*. A revocation
test that reports success because the harness is broken is worse than no test.

## 2. The epoch construction maps onto the library

`update()` rotates the group secret **without a membership change**, so a content
key per UTC day is expressible: nine manual `update()`s, one per epoch boundary.
§7.2's construction is not something p2panda has to be talked into.

**And revocation is finer than the epoch.** `remove()` rotates immediately, so a
revoked reader is cut off at the removal, not at the end of the day. The
reference's *"they keep up to the rest of the epoch"* is the pessimistic bound;
p2panda does better. This is the answer to the granularity question §7.2 left
open, and it is the good direction.

## 3. History on join is all-or-nothing — and §8.4 claims otherwise

```
cohort    holds 11 of the subject's 11 secrets
cohort    opens epochs [0..9]        ← every epoch, including six before it existed
```

feasibility.md §8.4 says the design *"can withhold history from a researcher
while granting it to a clinician"*, "because it is a **choice** per join".
**In 0.7.1 it is not a choice.** `add()` takes no subset argument and welcomes
the joiner with the whole `SecretBundle`.

The obvious workaround — trim the bundle with `update_secrets()`, add, restore —
was tried and **produced a member with zero secrets**, not a windowed one. That
may be this spike's harness rather than a hard limit, so the honest statement is:
*the supported API offers no way to scope history on join, and the unsupported
route did not work.*

**Consequence for the design.** Time-scoping has to come from **group
partitioning** — a cohort with a 90-day window gets its own group, whose bundle
only ever contains that window's secrets — which is §7.2's *"purpose scoping is
the same trick twice"* applied to time as well as purpose. That is more groups to
manage than the design assumed, and it is the one place where the reference
implementation is strictly more expressive than the library: per-recipient
wrapping scopes by *which keys you wrapped*, so a window is free.

## 4. Two practical notes

**It needs rustc ≥ 1.96.** This machine's stable was 1.94, and cargo did not fail
— it silently resolved to **0.6.1**, a superseded release. The spike pins
`=0.7.1` and a toolchain file so that cannot happen quietly again. Worth carrying
into the Android side: the NDK build chain inherits this floor.

**The integration surface is the cost, exactly as §8.4 said.**
`EncryptionGroup<ID, OP, PKI, DGM, KMG, ORD>` takes six generic parameters — key
registry, group membership, key manager, message ordering. This spike used the
crate's own `test_utils` implementations. **A shipping integration supplies all
four for real**, and that, not the cryptography, is the work.
