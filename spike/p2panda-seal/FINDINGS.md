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

---

# Follow-up, 2026-09-10: can "with history / from now on" be a choice per grant?

§3 concluded that time-scoping "has to come from group partitioning" and left it
untested. Two more spikes, `--bin windows` and `--bin addupdate`.

## 5. A window is a group, and forward-only works

`windows.rs` runs one `EncryptionGroup` per grant window. Sealing goes into the
current window; granting *from now on* opens a new window with that reader as an
initial member; a new window takes every existing reader with it.

```
    bob     in windows [1, 2]     opens epochs [3, 4, 5, 6, 7]
    carol   in windows [2]        opens epochs [7, 8, 9]

  'from now on' withholds earlier epochs       PASS
  a new window does not cut existing readers   PASS
  revocation still cuts immediately            PASS
```

Bob was granted at epoch 3 and opens nothing before it. Carol was granted at 7
and opens nothing before that. Bob was revoked at 8 and stops at 7 while carol
continues. **Forward-only grants, and revocation, come entirely from the library
— no wrapping, no key custody, no bespoke cryptography.**

Cost measured: 3 windows over 10 epochs, 15 control messages, and one group
state per window on the phone.

## 6. ~~THE BLOCKER: an added member goes stale~~ — CORRECTED, and it was the harness

> **Corrected the same day by [`../p2panda-spaces/FINDINGS.md`](../p2panda-spaces/FINDINGS.md) §2.**
> The suspect this section named — `test_utils`' DGM and message orderer — was
> the cause. On `p2panda-spaces`, which supplies the real implementations of
> both, a reader added mid-group opens the days published before the add AND
> every day after it. There is no blocker, and retroactive grants are a method
> call.
>
> What is below is left as written because the measurement was real and the
> reasoning is what a reader needs in order to trust the correction. **It is a
> finding about the test utilities, not about the library.**

### What was measured

`add()` is the only way to give a reader history, and a reader added that way
receives the secrets that exist at that moment and **then never advances**.

`addupdate.rs` is the smallest case that shows it — no windows, no catch-up, no
replay, everyone present from the start, zero refused messages:

```
  create(subject, [bob]) → seal 0 → add(alice) → update → seal 1 → update → seal 2

    bob     3 secrets, opens days [0, 1, 2]
    alice   1 secrets, opens days [0]
    subject 3 secrets
```

Alice opens day 0 — sealed *before* she was added — and nothing after. Bob, whose
only difference is that he was an *initial* member rather than an added one,
tracks everything. **This is the exact inverse of what a grant should do:** all
of the past, none of the future.

§1 did not catch it because its late joiner was added after the final epoch, so
there were no later rotations for it to miss.

**Root cause not established.** What was ruled out, by measurement rather than
reasoning: it is not delivery order (a fresh state with the whole control log
replayed into it gives the same answer), not a swallowed error (zero refusals on
both paths), and not the sender failing to process its own control message
(`Group::add` → `process_local` → `Dcgka::process` → `DGM::add` does update the
adder's membership view, and feeding the message back is refused as a duplicate).
What remains untested is the part §4 already warned about: **these spikes use the
crate's `test_utils` DGM and message orderer, and a shipping integration uses
neither.**

## 7. What this changes

`p2panda-store` 0.7.1 depends on `p2panda-encryption` and `p2panda-auth` and
ships **SQLite implementations** of `key_secrets` (`PreKeyBundlesState`),
`key_registry`, `groups` (the auth CRDT state) and `orderer`. §4 said "a shipping
integration supplies all four for real, and that, not the cryptography, is the
work" — that is now largely out of date, and it also means §6 must be re-measured
against the real DGM and orderer before it is treated as a limit rather than an
artefact of `test_utils`.

**The next experiment is `addupdate.rs` with `p2panda-auth` + `p2panda-store` in
place of `test_utils`.** If an added member tracks updates there, "with history"
is free and the design needs nothing bespoke. If it does not, retroactive grants
need an upstream fix, and the shippable subset is forward-only grants — which
§5 shows already work.
