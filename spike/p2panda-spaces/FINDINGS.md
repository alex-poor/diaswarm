# p2panda-spaces 0.7.1, measured

**Run:** `cargo run --manifest-path spike/p2panda-spaces/Cargo.toml`

The question: how much of [`crates/diaswarm-core/src/vault.rs`](../../crates/diaswarm-core/src/vault.rs)
and [`seal.rs`](../../crates/diaswarm-core/src/seal.rs) — 1,159 lines of sealing,
per-reader wrapping, grant log and key custody, none of it reviewed — does the
library already do?

**Answer: all of it.** Every grant this project offers is a method call.

---

## 1. The grant matrix, on the shipping layer

```
  bob   (added mid-window A, then revoked) opens ["A day 0", "A day 1", "A day 2", "A day 3"]
  alice (added to window B only)           opens ["B day 6", "B day 7"]
  stranger (granted nothing)               opens []

  1. an added reader opens EARLIER days           PASS
  2. an added reader opens later days             PASS
  3. a space is a window — no cross-space reading PASS
  4. remove cuts what comes next                  PASS
  5. a stranger processes everything, reads nothing PASS
```

| What the design needs | What it costs |
|---|---|
| grant **with history** | `space.add(reader, Access::read())` |
| grant **from now on** | create a new space; add the reader to that one |
| **revoke** | `space.remove(reader)` — rotates immediately |
| a holder that cannot read what it holds | nothing; it falls out |

Bob was added *after* days 0 and 1 were published and opens both, so history on
join is a property, not a workaround. He opens days 2 and 3 published after the
add. He was then removed, and days 4 and 5 are refused with *"tried to decrypt
message with an unknown group secret"* — the removal rotated the secret, and no
further wrapping is needed to enforce it.

Alice is in space B only. Given **every** message from both spaces she opens
B's days and none of A's. A space is the window; a reader granted "from now on"
is simply not a member of the space holding the earlier days, so there is
nothing to withhold and no per-segment key custody to get right.

The stranger processed all 21 messages of both spaces without error and read
nothing. **That is the property the whole architecture rests on**, and it comes
from the library rather than from `vault.rs`.

## 2. This CORRECTS `../p2panda-seal/FINDINGS.md` §6

That spike measured "an added member does not track later updates" — a reader
given all of the past and none of the future, which would have made retroactive
grants unusable. It said the root cause was not established and named the
untested suspect: it drove `data_scheme::EncryptionGroup` directly, with the
crate's `test_utils` DGM and message orderer in the two hardest generic slots.

**That was the cause.** Question 2 here is the same scenario on
`p2panda-spaces`, which supplies the real `GroupMembership` and `Ordering`
implementations, and it passes. §6 describes the test utilities, not the library.

The general lesson is the one §4 wrote down and this project then ignored for
one spike: `p2panda-encryption`'s test utilities are not a small stand-in for a
real integration, they are a different thing, and a property measured through
them is a property of them.

## 3. What still has to be written, and what does not

`p2panda-spaces` composes `p2panda-auth` (capability CRDT), `p2panda-encryption`
(the key layer) and `p2panda-store` (SQLite persistence for operations, key
secrets, key registry, group state and ordering). Above them it offers `Space`
with `add`, `remove`, `publish`, `members` and `repair`, and a `Manager` that
owns identity and key bundles.

**Not needed any more, if this is adopted:** the sealing construction, per-reader
wraps, the hash-chained grant log, the unlinkable grant tags, the on-disk vault
layout, and the `Have`/`Manifest`/`Grants`/`Segment`/`Wraps` protocol — messages
are `p2panda_core::Operation`s and replication is `p2panda-sync`'s log sync.

**Still ours, and now with a written reason:**

* **bucket assignment** — p2panda is topic-based and supplies no shard
  assignment. [`pool.rs`](../../crates/diaswarm-net/src/pool.rs) stays.
* **AAPS record canonicalisation** — nothing upstream knows what a bolus is.
* the JNI surface and the Kotlin plugin.

## 4. Two costs, neither of them fatal

**A reader must sync the subject's whole control history, not just its own
window.** Alice, fed only space B's messages, could not process even the first
one; fed both spaces in order she was fine. Two spaces created by one subject
are therefore not independent — B's auth history depends on A's. She still
cannot read A's data, so this is a metadata cost, not an access one: a
"from now on" reader learns that earlier windows exist and who is in them.
`D19`'s reduced-but-real leak, in a new place.

**`p2panda-auth` panics rather than erroring when an operation arrives without
its dependencies.** Observed at `group/crdt/mod.rs:727` (*"group already present
in states map"*, which fires when a group is *absent*) and
`group/resolver.rs:276` (*"all processed operations exist"*). Sometimes the same
situation returns a clean error instead — *"received space message … before auth
message, maybe it arrived out-of-order"* — so it is not consistently one or the
other.

This matters more here than in most applications: a diaswarm peer holds
ciphertext for strangers and will be handed messages out of order routinely.
p2panda-sync's log sync delivers in dependency order, which is the answer, but
**the integration must never hand an arbitrary message straight to the manager**,
and on a phone running a closed loop a panic in a background worker is not an
acceptable failure mode. The spike processes every message inside
`catch_unwind` for exactly this reason.

## 5. THE BLOCKER: a subject cannot have two spaces

Everything above uses one space per subject. The window design needs more than
one — "grant from now on" *is* a second space — and that does not work at 0.7.1.

```
  a reader in two of one subject's spaces

      adding to space A PANICKED (subject side)
  6. a reader can belong to two of one subject's spaces  FAIL
     opens ["B day 6", "B day 7", "B after both"]
```

A reader added to space B is fine. Adding that same reader to space A — which
already existed, and which the subject has been publishing to all along —
panics `p2panda-auth` at `group/resolver.rs:250`, *"all operations present in
map"*. **On the subject's side, not the reader's**, so it is not something a
careful reader can defend against.

**This is the library's own test API**, `add_persisted`, not
`crates/diaswarm-spaces`' hand-written persistence — so it is not our glue.

The likely shape of it: a subject's spaces share one **global** auth CRDT
(`Hash::digest(b"global-groups-context")` — one key, not one per space). Once
two spaces exist, their auth operations interleave in that single state and the
resolver's assumption that every referenced operation is present stops holding.
That is a guess about the cause; the failure itself is measured.

**Consequence.** `Reach::Everything` — grant with history — works and is
verified. `Reach::FromNow` is blocked, and with it the per-grant choice. Both
ways of opening the second window were tried: created with its members, and
created empty then added to. The first panics the reader once, the second three
times. Neither works.

**Not worked around, deliberately.** A workaround for a panic in a shared CRDT,
in a background worker on a phone driving an insulin pump, would be guessing at
someone else's invariants. This wants an upstream issue.

## 6. Version note

`p2panda-auth`, `p2panda-spaces` and `p2panda-store` are all published at 0.7.1
and pinned exactly here. `test_utils` is enabled on purpose: it supplies
`TestPeer`, which is a real `SqliteStore` plus the real DGM, orderer and forge —
not a stand-in for them.
