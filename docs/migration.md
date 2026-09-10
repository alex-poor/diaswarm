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

**1. ~~A backfill has never completed cleanly.~~ It does now, and the vaults
agree.** With thinning withdrawn, a full re-drain on the loop phone ran in
bounded passes with no crash, and **the shadow vault sealed exactly what it was
handed on every pass**. The two implementations have never once disagreed about
a record.

**2. ~~The device drains more than the snapshot tool sees.~~ Chased, and it was
a real difference between the vaults.** The device drained 30,188 CGM records
where `canon.py` yields 23,247. Measured on the snapshot, the database cannot
produce more than 23,247 *distinct* CGM records — version rows are byte-identical
once canonicalised, and no timestamp disagrees about a value. So 6,941 were
published twice.

The cause: AAPS's sync queue resolves every version row to the record it belongs
to, so one record arrives once per version row — 22,003 of them for CGM here.
The emitter drops repeats, but its memory lasts one pass, and a drain bounded
into passes hands the same record to several of them.

**That is where the two vaults differed.** `diaswarm-core` re-seals a whole
segment and skips what it already holds, *and* deduplicates again on read —
"a reader that trusted the segments to be disjoint would double-count insulin".
`diaswarm-spaces` appends, and had neither. A follower would have double-counted.

Fixed by deduplicating on read, where the old vault also does it, and where it
belongs regardless: a peer relaying two overlapping copies of a subject is
normal rather than broken. `a_record_sealed_twice_is_read_once` covers it, and
fails without the fix.

**3. Two phones replicate, partially.** Run on the real devices with
`crates/diaswarm-net/src/bin/twophone.rs` — a binary in `/data/local/tmp`, no
app involved:

```
carrying bucket for e9be07cbd8172011…
  +   4s  holding 1 operations
carried 1 operations from a phone it was never introduced to
opened  0 records — as it should: it was granted nothing
```

The claim holds: two devices on a real network found each other through the
pool, moved a subject neither was introduced about, and the carrier could open
none of it. What is *not* established is completeness — one of six operations
arrived inside the twenty-second window the binary waits. Whether the rest
follow, and how quickly, is unmeasured.

⚠️ **mDNS does not work from a bare binary on Android**: `ndk-context` panics
with "android context was not initialized", because local discovery needs a JNI
context the plugin has and a command-line process does not. These two phones
found each other anyway, but this harness is measuring a *worse* case than the
plugin would.

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

## Where it stands, 2026-09-10

**The vault migration is sound on its own terms.** `diaswarm-spaces` reproduces
exactly what it is given — 74 days of this subject's real history, on the phone
that produced it, sealed inside AAPS across app upgrades and restarts. Two real
phones replicate a subject over log sync and the carrier reads none of it. First
contact costs three seconds. Every remaining doubt is about something else:

* the drain emitting more than the snapshot tool sees (2 above), which affects
  the current vault identically;
* shadow mode not yet having run for days, across a reboot, a Doze window and a
  sensor change;
* two-phone replication proven to start, not to complete.

**What it would cost, now that nothing is thinned:** 54.9 MB a year per subject
and 329 MB for a peer carrying six, against 13.1 and 78 before. That is the
price of not losing readings, and it is the largest single consequence of this
work.

**Recommendation: do not cut over yet, and not because of anything measured
here.** Let shadow mode run for a week. If it still agrees pass for pass after a
reboot and a sensor change, delete `vault.rs`, `seal.rs` and `wire.rs` — the
evidence for doing so is stronger than the evidence that ever existed for the
code they replace.
