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

## 5. ~~THE BLOCKER: a subject cannot have two spaces~~ — WRONG, twice over

An earlier version of this section said a subject cannot have two spaces, and
recommended filing an upstream bug. **Both halves were wrong**, and the second
one nearly wasted a maintainer's time. What was measured was real; what it was
attributed to was not.

### 5a. Repair is the missing discipline, not a missing feature

Spaces share **one global auth state**. After any auth-level change — creating a
space, creating a group, adding or removing a member anywhere — every *other*
space is working from a stale view of it, and the next membership change on one
of them panics `p2panda-auth`.

`p2panda-spaces`' own `shared_auth_state` test does the right thing on every
line, with the comment *"Make Space 0 aware of this change"*:

```rust
let needs = manager.spaces_repair_required().await?;      // ask which are stale
let messages = manager.repair_spaces_persisted(&needs).await?;  // fix them
```

**Before every auth-level operation, not once before a batch.** Adding a member
to space A is itself an auth change, so space B is stale immediately afterwards.
With repair in place, every subject-side panic in this spike disappeared.

This was reached by reading the library's own tests rather than by reasoning
about the failure. Three attempts to reason about it produced three wrong
answers.

### 5b. A reader can belong to exactly one of a subject's spaces

What survives after repair is real and much narrower:

```
  6. a reader can belong to two of one subject's spaces  FAIL
```

Adding a reader to one space works. Adding the *same* reader to a second space
produces a space message carrying no welcome for them — *"expected direct
message of type 'welcome' but got nothing instead"* — and the reader then
panics processing what follows. Swapping which space is joined first swaps which
one works, so it is the **second** join that fails, not a particular space.

### 5c. The design that fits: a window per grant, and fan out

This does not sink windows, it reshapes them. Instead of carrying existing
readers forward into each new window:

* every reader stays in the **one** window they were granted in;
* the subject publishes each day into **every live window**.

```
  7. both windows keep receiving, each to its own reader  PASS
     alice now opens ["B day 6", "B day 7", "B after both", "B day 10", "B day 11"]
```

Alice is in window B only. Window A keeps publishing; she reads none of it, and
keeps receiving hers. Nobody is ever in two spaces, so 5b is never reached.

| Grant | How |
|---|---|
| **with history** | add the reader to window 0 — they get everything, onward |
| **from now on** | open a new window, add only them, publish into it from now |
| **revoke** | `remove` from their window |

**The cost is duplication**: one copy of every day per live window. With a
partner, a parent and a clinician on different terms that is three copies of
~78 MB a year rather than one. That is a real cost and it belongs in the
README's storage numbers, not buried here.

### 5d. What was nearly filed

An upstream issue, reproduced with LLM-written code, describing 5a as a defect.
p2panda's [LLM policy](https://github.com/p2panda/.github/blob/main/LLM_POLICY.md)
does not accept LLM-generated issues or code, which is what stopped it — but the
report would also have been **wrong**, and the answer was in their own test
suite the whole time.

## 6. Version note

`p2panda-auth`, `p2panda-spaces` and `p2panda-store` are all published at 0.7.1
and pinned exactly here. `test_utils` is enabled on purpose: it supplies
`TestPeer`, which is a real `SqliteStore` plus the real DGM, orderer and forge —
not a stand-in for them.
