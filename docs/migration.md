# Moving onto p2panda: what is proven, and what a cutover still needs

The vault, the grant model and replication have been rebuilt on p2panda's own
layers — [D20](decisions.md) and [D21](decisions.md). None of it ships. The old
implementation is still what runs on a phone, and this is the account of what
would have to be true before that changes.

It exists because "it works" is not a decision. The evidence below is specific
about what was measured, on what, and what each measurement does *not* cover.

---

## What is replaced, if this lands

| Today | Then |
|---|---|
| `diaswarm-core::vault` + `seal` — 1,159 lines of hand-composed cryptography | `p2panda-spaces`: `add`, `remove`, `publish` |
| per-reader key wraps, hash-chained grant log, unlinkable tags | the library's key layer and auth CRDT |
| `diaswarm-net::wire` — `Have`/`Manifest`/`Grants`/`Segment`/`Wraps`, polled | `p2panda-net` log sync, pushed after catch-up |

**SECURITY.md's headline warning is about the code this deletes.** That is the
point of the exercise, and it is the only reason to accept any of the costs
below.

Not replaced, with reasons rather than by default: `pool.rs` (p2panda is
topic-based and supplies no shard assignment), the AAPS record canonicalisation,
the JNI surface and the Kotlin plugin.

## What has been measured

| Claim | Evidence | What it does not cover |
|---|---|---|
| The same records come out | 31,341 records — 74 days of real history — through both implementations, identical | A migrated device, rather than a copy of its database |
| A holder cannot read what it holds | `tests/vault.rs`, `tests/replicate.rs` | Nothing; this one is the architecture and it holds |
| Revocation is prospective and immediate | `revoking_stops_what_comes_next`, and on-device | Long-run behaviour after many rotations |
| Grants can reach back, or not | `a_grant_reaches_back_only_when_it_is_asked_to` | Only after the fan-out rebuild; see D20 §5c |
| A peer carries a stranger's data knowing only a topic | `tests/replicate.rs`, on log sync | More than two peers |
| **The whole chain composes** — subject seals, stranger carries, granted reader reads *from the stranger* | `a_granted_reader_gets_a_subject_from_a_peer_that_is_not_the_subject`, reading out of the carrier's own store | Two processes, not two phones |
| It runs on the phone | Self-test binary on the loop phone: five days sealed in 31 ms, 2,016 records read in 181 ms | Anything inside AAPS |
| It runs *inside* AAPS | Shadow mode, hundreds of live passes, agreeing pass for pass | A full backfill without incident — see below |
| Identity survives a restart | Shadow passes either side of an app upgrade, different pids | A device reboot, a factory reset, a restore from backup |

## What is not yet true

**1. A backfill has never completed cleanly.** The first re-drain crashed AAPS
with an `OutOfMemoryError` — caused by shadow-mode code holding a second copy
of the whole history — and the second was interrupted by that crash's fallout.
Both causes are fixed and the drain is now bounded at 4,000 records a pass, but
**no full backfill has run start to finish**, so the on-device differential is
still unproven at full volume.

**2. 563 records are unexplained.** The device emitted 35,897 where `canon.py`
produced 31,341 from the same database. 3,898 was CGM thinning, 1 was a
post-emit edit. Ruled out: extra filtering, superseded versions, retracted rows,
a profile-table mismatch. The per-kind counters that would settle it have never
run over a complete drain.

**3. Nothing has replicated between two real phones.** The full chain now works
end to end — a granted reader opens a subject's history taken from a stranger's
store, and the stranger opens none of it — but in one process. The pool has only
ever been two phones on the old transport.

**4. There is no migration path for an existing vault.** A phone with 74 days
sealed the old way has to re-seal from the AAPS database, which is what the
re-drain button does — see (1).

**5. First contact is worse — by three seconds.** ~~An open question.~~
Measured in `spike/p2panda-logsync --bin firstcontact`, five cold runs each with
a fresh network id so no discovery state carries over:

| path | median | spread |
|---|---|---|
| direct dial — what an invite does today | **32 ms** | 29–33 ms |
| log sync — subscribe and wait for discovery | **3 s** | 2–6 s, 5 of 5 arrived |

A hundred times slower as a ratio, and three seconds as an experience. That is a
spinner, not a person scanning the code again because it looks broken — and the
follower UI already shows every reading with its age, so "waiting for the first
sync" is a small addition rather than a new idea.

**Recommendation: accept it, and delete `wire.rs`.** Keeping a bespoke fetch
path alive to save three seconds is the opposite of what this migration is for.

*Measured on a LAN with mDNS, both peers on one machine — which is the scanning
case, since people scan a code standing next to each other. First contact
between peers on different networks, relying on n0 discovery, is not measured
and could be slower.*

## A caveat about the evidence itself

**The network tests are timing-dependent.** Everything in `tests/replicate.rs`
and `tests/swarm.rs` waits for mDNS discovery — up to sixty seconds — and under
load that is not always enough: running all four crates' suites at once has
failed one of them twice, and both passed immediately on their own.

That matters more than an ordinary flaky test, because these are the tests a
cutover decision would rest on. A green run means the property held; a red one
means either the property broke or the laptop was busy, and nothing in the
output distinguishes those. Worth fixing before the decision, not after.

## What the cutover would cost

* **Storage per subject is unchanged** — 13.1 MB/yr, and that depends on §3.3
  thinning, which is now load-bearing rather than defence in depth.
* **One copy of each day per live window.** A partner, a parent and a clinician
  on different history terms is three copies rather than one.
* **4 MB of APK**, already being paid: the JNI entry points make SQLite and the
  spaces stack reachable, so the linker keeps them whether or not they run.
* **Reads are quadratic in operation count.** Irrelevant at 79 operations for 74
  days, and a standing rule never to put bulk data through a spaces message.

## What would have to be true

In order, none of them yet done:

1. A full backfill completes on the loop phone without incident, and the
   per-kind counts match `canon.py`.
2. The 563 is attributed.
3. Shadow mode runs for several days, across a reboot, a Doze period and a
   sensor change, still agreeing.
4. Two phones replicate a subject over log sync, and a granted reader on one
   opens what the other sealed.
5. Someone decides whether (5) above — worse first contact — is acceptable, or
   waits for upstream.

Only then is deleting `vault.rs`, `seal.rs` and `wire.rs` a decision rather than
a leap.
