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
| It runs *inside* AAPS | Shadow mode, hundreds of live passes without a crash | ~~agreeing pass for pass~~ — that comparison was never made; a real one now exists, see below |
| **The new vault returns what it was handed, on a phone** | `shadow agrees — given 17, holds 17, missing 0, lost 0, failures 0` — phone B, 2026-09-12, 17 records over 4 epochs | A long run, and the loop phone |
| Identity survives a restart | Shadow passes either side of an app upgrade, different pids | A device reboot, a factory reset, a restore from backup |

## What is not yet true

**1. ~~A backfill has never completed cleanly.~~ It does now — but "and the
vaults agree" was never established, and this entry used to say it was.**

With thinning withdrawn, a full re-drain on the loop phone ran in bounded passes
with no crash. That part stands. What does not is everything after it.

🔴 **The agreement was a number compared with itself.** `spacesSeal` returns
`records.len()` — the length of its own argument — on success. The plugin added
that up and logged it, and nothing anywhere compared it to the other vault or to
anything the shadow vault actually held. "The shadow vault sealed exactly what
it was handed on every pass" was therefore true by construction and could not
have come out otherwise: a shadow vault that stored nothing at all would have
produced the same line. "The two implementations have never once disagreed about
a record" is not a measurement, it is a restatement of an identity.

What those hundreds of passes do establish, and it is not nothing: the vault
opens on the phone, seals without throwing, and does not destabilise AAPS or
the loop. That is a liveness result. It is not a correctness one.

✅ **Fixed 2026-09-12.** Shadow mode now seals into the `diaswarm-keys` vault
(D26 decided against spaces, so shadowing spaces was measuring the wrong thing),
**reads the epoch back off disk**, and counts records that did not come back.
The pass line says `shadow agrees` or `shadow DISAGREES` with the counts behind
it, and a disagreement logs at error rather than info. "Agree" means the new
vault returns what it was given — ground truth, rather than a second opinion
from the vault being replaced, and strictly stronger, because two vaults can
agree by losing the same record.

**And asking what it would have to compare is what found the bug** that made it
worth doing: `diaswarm_keys::Vault::seal` was `fs::write`, replacing each epoch
rather than appending to it, so every five-minute flush destroyed the day so
far. See D26. No test caught it because no test sealed an epoch twice, and no
shadow pass caught it because shadow mode was not looking.

✅ **And it has now run on a phone.** Phone B, 2026-09-12, the generation bump
forcing a full re-read:

```
swarm: shadow vault is generation 0, wanted 3 — re-reading everything
swarm: drained 16 — event=16
swarm: sealed epoch 20705, 1 records … 20706, 10 … 20707, 4 … 20708, 1
swarm: shadow agrees — given 17, holds 17, missing 0, lost 0, failures 0
```

That is the first on-device correctness evidence this migration has ever had:
the JNI resolves, `diaswarm-keys` creates a vault on Android storage, seals, and
returns every record it was given.

🐛 **And the first line it produced found a bug, which is the argument for
device runs in one sentence.** `given 17` where the core vault sealed 16.
`shadowPending` was appended to unconditionally while `flushShadow` returns at
once when the shadow handle is 0 — the default on every phone, including the one
driving a pump. So every record ever drained accumulated in memory, never
flushed, never freed; about 160 KB a day at this subject's rate, for the life of
the process. `openShadow`'s own comment records an out-of-memory crash in the
same area. Pre-existing, arrived with the five-minute batching, and invisible to
the desktop suite because the leak only exists in the configuration the tests do
not run.

Fixed, rebuilt, reinstalled, and re-measured:

```
swarm: drained 1 — event=1
swarm: sealed epoch 20708, 1 records
swarm: shadow agrees — given 1, holds 3, missing 0, lost 0, failures 0
```

`given 1` matches the core vault exactly — no leftovers. And `holds 3` is the
other fix proving itself: that epoch already held 2 records, one more was
appended, and the read returned 3. Before `seal` stopped replacing segments it
would have returned 1. **The append path is verified on a real device against
real files.**

⚠️ **What this is still not.** One phone, 17 records, four epochs, minutes. Not a
long run, not a full history, and not the loop phone.

**Decided 2026-09-12, for when it reaches the loop phone:**

* **Full backfill.** The generation bump re-reads the whole AAPS history through
  both vaults — ~74 days, 30,000+ CGM records — in bounded passes. It is the
  strongest evidence available: the entire history through the new seal path,
  including the merge-and-append behaviour at a scale phone B cannot reach. This
  is the current behaviour; nothing needs changing to get it.
* **Accept the storage doubling.** The shadow vault is a second sealed copy of
  every record, so the plugin's footprint roughly doubles for as long as it is
  on. Shadow mode is temporary and off by default, so the cost is bounded by the
  soak. **Delete the `keys/` directory when the question is settled**, or when
  the cutover makes the core vault the shadow instead.

Both were considered against bounding the backfill, pruning the shadow vault,
and not shadowing the loop phone at all. The narrower options all buy less load
by buying less evidence, and the thing being bought is the only reason the
feature exists. What it establishes is that
the mechanism works and now reports honestly; the soak is still ahead.

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

**3. ~~Two phones replicate, partially.~~ Completely, and the old figure was
measuring the wrong thing.** Run on the real devices with
`crates/diaswarm-net/src/bin/twophone.rs` — a binary in `/data/local/tmp`, no
app involved. Two cold runs, 2026-09-12, publisher on phone B and carrier on the
loop phone, both over the relay:

```
carrying bucket for f0faea6d65446a1f…
  +   2s  holding 6 operations
COMPLETE: 6 of 6 operations in 2s
carried 6 operations from a phone it was never introduced to
opened  0 records — as it should: it was granted nothing
ingest  refused 0 · held 0 · panicked 0
```

The second run: 6 of 6 in 4s.

**THE 1-OF-6 THIS USED TO RECORD WAS A HARNESS ARTEFACT, NOT A RESULT.** The
carrier stopped as soon as *anything* had arrived and twenty seconds had passed,
then printed what it held — which answers "did replication start", not "did it
finish", and this gate asks the second. It now takes the publisher's sealed
count and waits for it, so it can say COMPLETE or INCOMPLETE rather than leaving
a number to be misread. It also reports `refused`/`held`/`panicked`, because a
carrier that opens nothing because it was granted nothing and one that opens
nothing because operations are stuck waiting on a dependency look identical from
the record count alone.

Both runs also predate nothing: they are the first on the relay harness — the
old figure was taken before relays existed — and the first through the
dependency orderer.

*Not established:* the reverse direction, with the loop phone publishing. The
sandbox declines to run a long-lived background process on the phone driving a
pump, which is the right call and leaves that half unmeasured.

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

> **This list said "none of them yet done" while the section above it struck
> two of them through, and item 2 asked about a "563" that no longer appears
> anywhere in this document** — `1bdcffd` rewrote that finding around 6,941 and
> left the checklist behind. A gate list that disagrees with its own evidence is
> worse than no gate list, because it is the thing a cutover decision would be
> read off. Corrected 2026-09-12, and item 6 is new.

1. ~~A full backfill completes on the loop phone without incident, and the
   per-kind counts match `canon.py`.~~ **Done** — see (1) above.
2. ~~The drain excess is attributed.~~ **Done** — 6,941 records published twice,
   because AAPS's sync queue resolves every version row to its record and the
   emitter's memory lasts one pass. Fixed by deduplicating on read.
3. **Mostly done, and the word "agreeing" was never backed by code.** Measured
   2026-09-12, on the loop phone:

   * **Several days.** `spaces/credentials.json` is untouched since
     2026-09-10 15:47 and the vault has sealed continuously since, through the
     app upgrade to `diaswarm6`.
   * **A cold reboot.** Uptime was 4 days 22 h — it had genuinely never been
     restarted since the spaces vault was created. After `adb reboot`, AAPS
     restarted itself on `BOOT_COMPLETED` under a new pid, and
     `credentials.json`, `node.key` and `subject.id` are **byte-identical**.
     The vault reopened and sealed on the next pass. This is the condition with
     consequences: a device returning as a new member invalidates every grant
     ever made to it.
   * **Both sensor changes.** Not waited for — already in the history. The
     backfill covers epochs 20630..20708, which spans Dexcom G6 → a morning of
     Libre 2 → Libre 3 on 2026-09-07. A week of ordinary running might cross
     none; this crossed two.
   * **A Doze period.** Not done, and not attempted: forcing idle on the phone
     driving a pump is not worth the evidence it adds.

   ⚠️ **"STILL AGREEING" IS NOT WHAT SHADOW MODE MEASURES.** It seals into the
   second vault and adds to a counter — `shadowSealed += n`, logged as "shadow
   sealed N". Nothing reads back from either vault and nothing compares;
   `spacesStatus` has no call site outside its JNI declaration. Two doc
   comments in `DataSyncSelectorSwarmImpl` say "and say whether it agrees".
   They do not.

   So what a week of green shadow logs establishes is: it does not crash, hang
   or leak, its identity survives, and it accepts the same *count*. Agreement
   is owned by `tests/differential.rs`, on 31,341 real records, and the phone
   has never added to it. Building a live comparison needs a granted reader's
   private key on a desktop, which is why it has not been done.

   ⚠️ **AND THE LIVE PATH DEFEATS D20's BATCHING, WHICH IS NOT A DISK
   PROBLEM.** The shadow vault is 27 MB against the old vault's 11 MB;
   [D21](decisions.md) measured 3.2 against 8.3, so the ratio has inverted.
   Pulled and counted:

   | operations | count | body | header |
   |---|---|---|---|
   | under 2 KB | **4,596** | 836 KB | **1,929 KB** |
   | 2–32 KB | 796 | 12.8 MB | 334 KB |
   | over 32 KB | 124 | 7.6 MB | 52 KB |

   The large ones are the backfill, batched into 64 KB bodies exactly as D20
   intended: 0.7 % header overhead. The 4,596 small ones are **live sealing**,
   one operation per pass, median body **99 bytes** inside a ~430-byte signed
   header — 2.3× more header than payload, and 83 % of every operation in the
   vault.

   D20's conclusion was "never put bulk data through a spaces message", and its
   fix was to batch. The backfill batches. The live path does not: it seals
   whatever one pass accumulated, and with a one-minute sensor and a ~one-minute
   pass that is a single reading.

   **The cost that matters is not the megabytes.** D20 measured reads as
   quadratic in operation count — 5 ms per operation at 537, 155 ms at 10,569,
   twenty-seven minutes for 74 days — and concluded "at 79 operations for 74
   days it stops mattering". It is now **5,516 and climbing by roughly 1,400 a
   day**, from six weeks of one subject. Half way to the pathological figure,
   with no bulk data involved. Every one of those also replicates.

   Not fan-out: `windows` is 1.

   **BATCHING IS NOT THE FIX, AND MEASURING IT SAID SO.**
   `crates/diaswarm-spaces/src/bin/opcost.rs`, one seal per operation, which is
   the live shape:

   | ops | read | per op | process | persist |
   |---|---|---|---|---|
   | 125 | 0.24 s | 1.92 ms | 0.15 s | 0.06 s |
   | 250 | 0.62 s | 2.49 ms | 0.40 s | 0.17 s |
   | 500 | 2.36 s | 4.71 ms | 1.54 s | 0.69 s |
   | 1000 | 9.80 s | 9.80 ms | 6.44 s | 3.03 s |
   | 2000 | **43.77 s** | **21.88 ms** | 27.59 s | 15.28 s |

   Per-operation cost doubles as the count doubles, so the total is quadratic,
   and roughly two thirds of it is inside `p2panda-spaces`' own `process`.

   The decisive run is the second one. Delivering the same 2,000 operations in
   ten chunks of 200 took **42.98 s against 43.60 s all at once** — no
   improvement — and each chunk cost strictly more than the one before it, 0.43 s
   climbing evenly to 8.55 s. So this is not a per-call cost that incremental
   catch-up avoids. **The cost of processing an operation rises with how much
   history the vault already holds.** Every operation is dearer than the last,
   permanently, whatever schedule it arrives on.

   What that means for the three candidate fixes:

   * **Batching live seals** is still worth doing — cost is quadratic in the
     count, so cutting 1,400 operations a day to 288 is about a 24× saving. But
     it moves the wall by weeks, it does not remove it.
     **Done 2026-09-12**, in `flushShadow`: the spaces path accumulates across
     passes and seals every five minutes, or sooner for a bulk handover, or
     immediately once an epoch closes. Five because it is the cadence the loop
     reasons in and the bucket `spec/records.md` uses, so a reading waits at most
     one CGM cycle longer than it already did; an hour would be twelve times
     cheaper again and would make a follower useless at 3 a.m. On the loop phone
     the two cadences visibly diverge — the core vault sealing at 16:47:28,
     16:48:04, 16:48:24, 16:55:32 and the shadow at 16:47:28 and then 16:55:33
     with four records in one operation.
   * **Chunked or incremental catch-up** buys nothing. Measured.
   * **Pruning does not help either, and this was measured rather than
     assumed.** The cost is not in `operations_v1`. It is in `spaces_v1.state`,
     which grows linearly at about 500 bytes per operation and is read, mutated
     and written back on *every* operation — so processing N operations moves
     roughly N² × 500 bytes:

     | ops | `spaces_v1.state` | per op |
     |---|---|---|
     | 125 | 64,564 B | 516 B |
     | 500 | 251,977 B | 503 B |
     | 2000 | 994,233 B | 497 B |

     That state is p2panda's own structure and grows with every operation a
     vault has **ever processed**, whether or not the operation is still held.
     `LogStore::prune_entries` is implemented and callable, and dropping
     operations would change none of it.

     On the loop phone the *subject's* state is only 42 B/op. A reader
     accumulates roughly twelve times more, tracking decryption state per
     message, so the expensive side is the reading side.

### What that means per use case

At 288 operations a day, after the batching above:

| | window | operations processed | state | verdict |
|---|---|---|---|---|
| **Parent / partner** | 24 h, continuous | 288/day, **cumulative for ever** | 52 MB after a year | breaks, and it is the flagship |
| **Clinician** | 90 days, summarised | ~26,000 | 13 MB | ~1.5 h on a desktop, hours on a phone |
| **Research** | a year, or from a date | ~105,000 | 52 MB | ~27 h, but it runs on a gateway |

**The parent is the case this design serves worst, and 24 hours is not why.** A
day is trivial. The problem is that a parent who follows continuously
accumulates state for every operation they have ever ingested: they look at a
day and pay for the year.

**The answer is forgetting, not pruning — and that is measured, not asserted.**
A parent needs no history, and `Reach::FromNow` opens a window that cannot
contain what predates it. Against a subject holding 1,500 operations:

```
of 1,500 operations, 4 are auth-carrying; 1,496 are application messages

reader with all the history :  22.97s for 1,496 records
reader granted from now     :   0.89s for   289 records
```

**The auth history a new reader must process is four operations, not fifteen
hundred.** All of a subject's spaces share one global auth state
([D20](decisions.md)), so a reader joining a brand-new window still needs every
window creation and every grant — but those are a handful. The bulk is
application messages, and a `FromNow` reader is not a member of the window
holding them.

So a **new** reader in a **new** window pays for the auth chain plus the day it
actually watches: about 26× cheaper here, and the ratio grows with the history
being skipped.

⚠️ **AND THAT IS THE ONLY CASE IT WORKS FOR. An existing reader cannot shed its
state.** This section first claimed a parent could simply be "re-granted into a
fresh window", which does not follow — a new grant does not touch the reader's
own `spaces.sqlite`, and a parent who has followed for a year still has a year
of state in it. Tested directly: discard the state, keep `credentials.json`,
hand back the auth chain and a day —

```
kept identity, dropped state:  0.06s for 0 records (held 577)
```

Nothing. `SpacesArgs::Application` carries `space_dependencies` — the previous
tips of that space — so application messages form a chain, and a reader with no
state cannot process today without having processed yesterday. It must replay
the window from creation, which is the expensive thing it was trying to avoid.
The fresh reader above only worked because `FromNow` gave it a window with no
prior tips.

**Re-pairing was the wrong answer and is withdrawn.** Following your child is
permanent until revoked; a design that makes somebody rescan a code every few
weeks to keep watching is not a design. What was actually worth testing is the
move that needs no re-pair at all: the **subject** opens a new window and adds
the same reader to it, using the key it already holds.

Measured in `rotating_a_window_resets_what_a_long_standing_reader_pays`, at
1,505 operations of history:

```
a day before rotation :  0.032s
a day after rotation  :  0.230s      ← seven times worse
```

**Rotation makes it worse.** `Vault::seal` publishes into every window
unconditionally — `for n in 0..self.windows` — with no test for whether anybody
reads that window. A second live window doubles the publishing rather than
resetting the cost, which is D20's per-window fan-out arriving where it was not
expected.

### So the shape of the problem, stated plainly

A permanent follower who reads a fixed recent window is the flagship use case,
and the vault has no way to express it:

* reading today requires having processed the chain to today, because
  `SpacesArgs::Application` depends on its space's previous tips and the
  decryption state ratchets forward. The marginal cost of one operation is
  roughly 21 µs × the history already processed — 0.032 s at 1,505 operations,
  and seconds each at a year's worth;
* a reader cannot join a window part-way, so it cannot skip what it does not
  want;
* a window cannot be closed, so rotating into a fresh one multiplies the
  publishing instead of replacing it;
* and discarding state loses the chain entirely (`held 577`, above).

None of those four is fixable in this repository. Three are properties of
`p2panda-spaces`; the fourth — publishing into windows nobody reads — is ours
and is a small change, but it only helps once one of the others gives.

### And the vault being replaced does not have the problem

Measured with `crates/diaswarm-core/src/bin/readcost.rs`, a day of a five-minute
sensor per epoch:

| days held | whole vault | recent day only | records |
|---|---|---|---|
| 7 | 0.004 s | 0.001 s | 2,016 |
| 30 | 0.013 s | 0.000 s | 8,640 |
| 90 | 0.041 s | 0.001 s | 25,920 |
| 180 | 0.082 s | **0.001 s** | 51,840 |

**Reading the recent end is flat**, whether the subject holds a week or six
months, and a full catch-up is linear rather than quadratic — 51,840 records in
0.082 s. The spaces vault took 22.97 s for 1,496.

It is structural, not luck. Segments are sealed independently per epoch, named
by epoch, each under its own key wrapped per reader — so opening epoch N needs
segment N and its wrap and nothing else, and `read_as_from` discards older
segments by filename before decrypting anything. There is no chain to walk and
no state that grows.

So the migration as it stands would trade a vault that does exactly what the
flagship needs for one that cannot express it. That is not an argument against
`p2panda-spaces` — it deletes 1,159 lines of unreviewed cryptography, which is
the whole point ([D20](decisions.md)) — but it is the cost, and it was not on
the list before today.

The alternative is truncating a window, which would need `space_dependencies`
to tolerate a gap. That is upstream's territory and does not exist for spaces.

*One wart, recorded rather than solved:* the fresh reader ends with 290
operations `held` — the earlier window's application messages, waiting on
dependencies it will never be given because they are not for it. Harmless, but
a vault accumulating permanently-pending entries wants a way to discard them.

**The clinician is a shape problem, not a scaling one.** Summarised statistics
require reading all the raw data to compute them, and the alternative — the
subject publishing summaries — is what [D6](decisions.md) forbids: `tdd` was
removed from the vocabulary precisely because a derived field in a stream
becomes a second source of truth. A one-off desktop ingest, or a rethink.

**Research is the only one the current shape already fits.**
[D5](decisions.md)'s commons gateway is always-on institutional hardware doing
occasional runs, and `Reach::FromNow` already expresses "from a start point
onwards".

Whatever else is true, the auth and grant operations must never be dropped —
[D13](decisions.md) makes their permanence the thing that stops a subject
quietly shortening their own grant history.

   For scale: the loop phone holds 5,516 operations after six weeks and gains
   about 1,400 a day. At that depth a single day's catch-up is already tens of
   seconds on a laptop and minutes on a phone, and it grows. This is a
   cutover blocker, and it is worth reporting upstream, since most of the time
   is theirs.

   ~~Out-of-order arrival loses records silently.~~ **Closed 2026-09-12.**
   `ingest` used to process in arrival order and catch the resulting
   `p2panda-auth` panic, so an operation that arrived before its dependency was
   not delayed — it was dropped, and its records with it, leaving a count as the
   only trace. It now queues through `p2panda-store`'s `OrdererStore` and
   reports `held`, which is a different sentence from `panicked`: stored,
   pending, released when the dependency lands. `a_shuffled_bundle_still_reads_completely`
   and `a_dependency_arriving_late_releases_what_waited_for_it` cover it, and
   both fail if the ordering is removed.
4. **Half done.** Two phones replicate a subject over log sync completely —
   6 of 6 in 2s and 4s, twice cold, over the relay, with a carrier that opens
   none of it. What is still untested is the other half of the sentence: a
   *granted* reader on one phone opening what the other sealed. `twophone`'s
   carrier is granted nothing by design, so it proves carriage and not reading.
5. ~~Someone decides whether worse first contact is acceptable.~~ **Answered**
   — three seconds, accept it.
6. **Not done, and nothing else on this list knew about it.** An invite has to
   be enough to grant somebody. `p2panda-spaces` needs a long-term **key
   bundle** as well as a public key, and `diaswarm:2:` carries no such field —
   so on the spaces vault, scanning a QR code cannot result in a share. The
   shipping vault has no equivalent need: `vaultGrant` takes hex and wraps
   straight to it, which is why sharing works on hardware today.

   `a_grant_needs_more_than_the_key_an_invite_carries` holds this down.

   **AND THE FIX IS NOT A BIGGER INVITE.** That was the obvious reading and it
   is wrong: `Member` derives only `Debug`, is not constructable from outside,
   and carries a note explaining why — *"this struct does not guarantee if the
   member's handle / id is authentic ... care will be required as soon as
   `Member` gets constructable, serializable etc."* A bundle pasted into a QR
   string is a bundle nobody signed, and an impersonation waiting to happen.

   The supported path is a **signed operation**: a peer publishes its bundle as
   `SpacesArgs::KeyBundle` and whoever ingests it registers that member as a
   side effect of processing a message the author signed. Authenticity comes
   from the signature, not from trusting the channel.

   Measured in `a_published_key_bundle_makes_a_bare_key_grantable`: a grant from
   a bare key fails, one signed operation is ingested, the same grant succeeds,
   and the reader opens what was sealed after it. `Vault::key_bundle` exposes
   the message; `Vault::key_bundle_expired` is the rotation check nothing calls
   yet.

   **So the invite format does not change.** What is left is carrying one extra
   operation at first contact, and there is already a channel for it — the
   subject dials the follower during the one-scan exchange ([D25](decisions.md))
   and the follower hands its invite back. That is where the bundle should
   travel, and it is the wire work this gate now reduces to.

   Found by reading upstream rather than by running anything: `Vault::register`
   takes another `Vault`, which only ever exists with both peers in one
   process, and its doc comment claims "on a phone this is what scanning an
   invite does".

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

## The cutover, and the one thing blocking all of it

**Decided 2026-09-12: a cutover is wanted, and `diaswarm-keys` is not close to
one.** It is a storage engine wired into exactly one caller — shadow mode
sealing. The shipping path is entirely `diaswarm-core`:

| Piece | Today | Needed |
|---|---|---|
| Seal | `vaultSeal` → core vault | ✅ keys seals, shadow only |
| Grant | `vaultGrant` → `record_grant` + `publish_wraps` | ❌ no JNI for `keys::Vault::grant` |
| Invite | `subject, endpoint, purpose, relay` | ❌ **carries no key bundle** |
| Transport | `wire.rs` pull protocol, polled | ❌ `KeysReplicator` tested, nothing calls it |
| Ayni read | `netGlucose`/`netTreatments`/`netLatest` → core vault | ❌ rebuilt on keys segments |
| Existing data | 74 days, followers already granted | ❌ **a naive cutover invalidates every grant** |

That last row is the sharp one. Following is meant to be *permanent until
revoked*; a cutover that drops existing grants means re-pairing, which has
already been rejected as an answer.

### Why the bundle exchange is the blocker

`p2panda-encryption` agrees keys from a `LongTermKeyBundle`, and **both sides
need the other's before a grant can exist**: the subject calls
`grant(reader_bundle, purpose)`, and the reader needs the subject's bundle to
derive the same `GrantTag` and to open its welcome. The invite carries neither.
So today there is no way to grant a reader on the keys vault *at all* — which is
why nothing downstream of it can be tested end to end, and why it is the first
thing to build.

The channels already exist and only need a field each:

* the **invite** (subject → reader) carries the subject's bundle. New invite
  version; `diaswarm:2:` must keep working for the core vault.
* **`Request::Offer`** (reader → subject, D25) carries the reader's bundle. It
  already exists precisely so one scan finishes the exchange both ways (D16).

A bundle is a few hundred bytes, so a QR code is unbothered.

### Next session: unblock, do not cut over

1. `keys::Vault` JNI beyond sealing: `keysBundle` (ours, for an invite),
   `keysGrant(reader_bundle, purpose) -> tag`, `keysRevoke(tag)`,
   `keysJoin(subject_bundle, subject_key, purpose)`. **Each needs a
   `SqliteStore` alongside the vault**, because a grant is only real once
   `wire::publish_control` has put it in the control log.
2. Bundle into the invite and into `Request::Offer`, with the old invite still
   parsing.
3. Prove it between two phones with a `twophone`-style binary on the keys vault
   — `crates/diaswarm-net/src/bin/twophone.rs` is the existing harness and runs
   from `/data/local/tmp`, nowhere near AAPS.

Only then is the cutover a sequence of testable steps rather than a cliff. The
migration of existing followers is a separate question and is still unanswered.

### ✅ Unblocked, 2026-09-13 — and proven on two phones

`Vault::my_bundle` returns the identity a vault actually holds, and
`encode_bundle`/`decode_bundle` put it in text. `crates/diaswarm-net/src/bin/twokeys.rs`
is the proof, and it ran on the two phones:

```
  (phone B)  granted e33147ca11e8ea74…
             sealed  3 days
  (loop phone)  following 617a18315c06ea97…
                joined after 4s, as e33147ca11e8ea74…
                READ 36 records from 3 segments, off a phone it was never introduced to
```

The tag the follower derived is the one the publisher granted, and neither side
ever held the other's `LongTermKeyBundle` as a value until it had been through a
string. The bundles travelled on the command line, which is not a shortcut: it
is exactly the two hops the real thing uses — the subject's in the invite, the
reader's in `Request::Offer`. Moving them into those fields is app plumbing;
whether the protocol works at all was this.

**A follower needs one identity and several vaults**, which is not obvious and
would have broken the JNI silently. A `Vault` holds one group state, so
following three people means three vaults — but the bundle the device published
belongs to *one* key manager, so all three must join with that same manager.
A vault that minted its own would be a different member to the one that was
granted, and would read nothing while looking healthy: no error, no crash, an
empty graph. `Vault::manager_state` exists for this and says so.

⚠️ **`ndk-context` panics in a bare binary** — "android context was not
initialized", raised inside a tokio task, caught, the task dies and the swarm
carries on without network-change detection. Already documented in
`crates/diaswarm-android/Cargo.toml`; harmless for a wifi test, and not a new
problem.

### ✅ The publisher side, on a phone — 2026-09-13

Everything but Ayni is now wired and was checked on phone B after installing:

```
swarm: in the pool as b8e0c9badf6b…
swarm: shadow agrees — given 1, holds 2, missing 0, lost 0, failures 0
swarm: keys carrying 1 log(s)
diaswarm:3:801c9be0…:b8e0c9ba…   (8 fields — the invite carries the keys identity)
```

The shadow line is the one that mattered. `keysOpen` now **borrows the pool's
SQLite store** rather than opening its own, because `p2panda-store` builds pools
with `max_connections(1)` and no busy timeout — two pools on one file would make
sealing and replication take turns failing. That it still agrees says the
borrowing works on a device, which was the day's main risk.

`keys carrying 1 log(s)` is this phone's own log. The one subject it follows was
paired before the keys field existed, so it is skipped rather than failed —
documented behaviour, and visible because the count is logged.

🐛 **And the first version of that line logged only failures**, so a pass that
carried everything and a pass that carried nothing read identically. Written by
the author of a day spent removing exactly that shape from other people's code.

**Remaining for a cutover:** Ayni — it has no keys vault, hands out a v2 invite,
and cannot complete a D26 pairing; and D27's existing-follower migration, which
is designed and unbuilt.

### ✅ The full history, on the phone that drives the pump — 2026-09-13

77 epochs (20630..20709), roughly 30,000 records of this subject's real
history, re-drained through both vaults with a comparison that compares:

```
swarm: shadow vault is generation 0, wanted 3 — re-reading everything
swarm: drained 4000 — cgm=4000
swarm: shadow agrees — given 4003, holds 4004, missing 0, lost 0, failures 0
swarm: drained 4000 — cgm=4000
swarm: shadow agrees — given 4000, holds 4148, missing 0, lost 0, failures 0
```

**The outcome is on disk and does not depend on the log**: the keys vault holds
`20630.json` … `20709.json`, **77 segments for 77 epochs**, beside a 810-byte
`group.cbor` and its SQLite store. Storage is **12M against the core vault's
12M** — the doubling that was accepted, measured rather than estimated.

⚠️ **What this does not prove.** The logcat ring buffer rotated during the
backfill, so the verdicts from the middle passes are gone. Every verdict
actually seen said `missing 0, lost 0, failures 0` and none ever said
`DISAGREES`, but "every pass agreed" is not something this run can claim — only
"every pass observed agreed, and all 77 epochs arrived". The buffer is now 16 MiB
so the next run is fully observable.

🐛 **And the first attempt was wasted by a bug the test phone hid.** `swarmJoin`
never created `files/diaswarm/keys/`, and SQLite creates a database file but not
the directory holding it — so the store failed to build, there was no
replicator, and every pass said `keys carry unavailable (-3)` and `shadow vault
would not open`. Phone B had passed because an *earlier* build's `keysOpen` made
the directory as a side effect; a device that had never run that build failed
every time. The generation bump had meanwhile consumed the backfill into the
core vault and marked itself done, so the re-drain had to be triggered by hand
afterwards.

### ✅ D27 on two phones — an existing follower moved without re-pairing

```
(ayni)     handed over to 552f688a
(subject)  handed over a reader as 265a21af855bdaf6… (follow)
```

`265a21af855bdaf6…` is **the same tag the manual pairing produced earlier that
day**, so the handover path derives the identical relationship identifier as a
scanned one. Nobody scanned anything: the reader offered its keys identity with
a proof only the two of them can make, and the subject granted it.

⚠️ **The keys read path lags the core one** — 1–3 minutes against 45s–2min,
because replication adds a hop the local vault does not have. Not a bug and not
yet closed.

🐛 **And the follower's fallback was wrong, found by somebody watching their own
graph.** It fired only on an *empty* keys read. A read that returned 221
readings with a two-hour hole and a stale tip is not empty, so it never fired
and the screen showed a broken history that looked deliberate. The hole was real
— shadow mode had been off on the subject for two hours — and a migration spends
its whole life in that condition, one vault complete and the other filling in.

Fixed by not choosing: both vaults are read and merged on timestamp. "Is the
keys read good enough to prefer?" is the wrong question, and every threshold
answering it — non-empty, fresh enough, long enough — is wrong for somebody.

### ✅ CLOSED — a follower read real glucose out of the new vault, 2026-09-13

Two phones, nothing shared between them but two invites:

```
(loop phone)  swarm: keys granted as 265a21af855bdaf6…
              swarm: shadow agrees — given 1, holds 864, missing 0, lost 0, failures 0
(ayni)        keys carrying 2 log(s)
              keys read 407 reading(s) for 552f688a
```

On screen at that moment: **6.8 mmol/L, 47 seconds old**, with the chart and the
basal trace — all of it opened from segments that arrived as p2panda operation
bodies, under a secret handed over in a welcome the follower found for itself.

The whole chain, each link on real hardware: AAPS seals into the keys vault and
publishes each segment as a signed operation · hands out a v3 invite carrying
both of its keys · scans the follower's invite and grants it on the keys vault ·
the grant is a signed control message in a replicated log · Ayni carries both
logs, finds its own welcome among nine grants without any message naming a
recipient, derives the same `GrantTag` independently, joins with the identity it
published, and reads.

**The cryptography was never the problem.** The pairing — unlinkable tags
derived separately on two devices — worked first time. Every failure was
plumbing at a seam:

| | |
|---|---|
| `seal` replaced the day instead of appending | no test sealed an epoch twice |
| `shadowPending` grew unbounded while disabled | the leak only exists when the feature is off |
| the v3 invite carried one key of two | the log author and the bundle are different keys |
| `netFollowing` omitted the keys column | the follower had a bundle and no log to fetch |
| `swarmJoin` never created the keys directory | the test phone had it from an earlier build |
| a grant decoded an identity as a bundle | every test encodes and decodes with the same pair |
| segments were sealed but never published | files on one phone, nothing in the log |
| one epoch became many operations | "last N entries are the last N days" stopped holding |

Eight, in a suite of ~57 that was green throughout. Each lives between two
components; none is visible to a test that exercises one.

⚠️ **Only the loop phone can demonstrate the whole thing**, because it is the
only device with records to seal. `twokeys` proved the protocol between both
phones; the *app* path — a v3 invite out of AAPS, scanned, granted, replicated,
read — needs a publisher with data, and now has one.

### ✅ CLOSED — treatments, 2026-09-13

The glucose reader shipped first and by itself it looks like success: the line
draws, the number is fresh, nothing errors. `keysGlucose` was the whole reader,
so a subject moving to the keys vault would have kept their graph and silently
lost **every bolus, carb, TBR and extended bolus** from it. The emitter had
always drained all four kinds; only the reader was half-built.

`keysTreatments` is the other half, and both readers now build their rows from
one `treatment_line`, so the chart cannot change underneath somebody because
their records arrived by a different route. Measured on phone B:

```
merged 401 core + 224 keys = 401 for 552f688a
merged 169 core +  63 keys = 169 treatments for 552f688a
```

The totals do not move, which is the point: every row the keys vault produced
was byte-identical to a row the core vault already had. A single disagreement in
rounding, field order or the TBR `abs` flag would have shown up as 170.

`abs` is the field that earns its own test. It is not a value, it decides what
`rate` **means** — 150 is either 150% of basal or 150 U/h — and a reader that
dropped it would draw a plausible chart that was wrong by a factor of a hundred.

### 🐛 The switch did nothing, in the direction nobody looked

`swarmJoin` decides **once**, at join, whether the pool gets a keys replicator,
from the directory it is handed. Nothing later can add one. Turning the
preference **on** therefore changed nothing at all until the process restarted —
and the logs blamed the wrong thing while it did:

```
keys carry unavailable (-3)          (Ayni)
swarm: shadow vault would not open   (AAPS)
```

Both read as a broken vault. Neither is: they are a pool that was never asked
for one. This is the same defect as the latency regression written up above —
which was a replicator that kept running after the switch went off — just
failing in the other direction, where nothing gets slower and nobody notices.

Ayni restarts its endpoint when the switch moves. AAPS compares the running pool
against the preference on every pass, which is also right when the preference is
changed somewhere the plugin never hears about, and costs one boolean a minute.
On phone B: off at 14:43:25, on at 14:43:37, `keys carrying 2 log(s)` on the
pass four seconds later, no relaunch — and the screen still **1 min ago** with
the replicator running, so the latency fix holds with the feature on.

### 📊 What it actually costs, measured on the loop phone

After four hours of shadow sealing and publishing at a one-minute cadence:

| | |
|---|---|
| local merged day file | ~520 KB/day (`keys/segments/20709.json`) |
| published log | 36 ops, 137 KB bodies + 11 KB headers in ~4 h → **~870 KB/day** |
| the 77 backfilled epochs on disk | 12.1 MB |
| `spaces.sqlite`, written to last at 10:19 | 28.1 MB |

So the keys path costs about **1.4 MB/day** all in, roughly 1.7× the plaintext
day — which is the delta fix holding in practice, against the 144× a merged-day
publish was costing before it.

Two things the numbers say that the code does not:

⚠️ **The 931 KB WAL is not 931 KB of content.** `keys.sqlite` is 4 KB with a
931 KB `-wal`; the log inside it is 148 KB. A WAL holds every rewrite of a page
until it checkpoints, so reading the file size as a growth rate overstates it by
about six times.

⚠️ **The backfill was never published, so no follower can read it.** 77 epochs
are sealed on disk; the log holds 36 operations, all from after shadow
publishing started this morning. A follower that joins today sees from 10:26,
not from 77 days ago. That happens to match the standing decision — a parent
needs 24 hours, not a history — but it is an accident of what publishes, not a
rule anything enforces, and `seal_checked` publishing only the current epoch's
delta is the reason.

🧹 **28 MB of `spaces.sqlite` is dead.** D26 dropped the spaces message layer;
the file stopped being written when the keys build landed and nothing reads it.
It is the single largest thing this project has put on that phone. Deleting it
is a one-line cleanup, and it is the user's data on the user's loop phone, so it
waits for them to say so.

### 🐛 The third reader, and the day that could not explain itself

Two readers were built — glucose and treatments — and both agree. The third was
found by asking what else the screen shows: the target band, and the scheduled
basal. Both come from a `profile` record, both read the core vault only, and
`scheduledBasal` answers **0.0** when there is none.

That is the reader whose absence misleads rather than shows. A missing line is
visible. A percentage temp basal resolved against a basal of zero draws a
confident flat trace that is wrong by whatever the basal was.

`keysProfile` fixes the reader — every epoch, not a tail, because a profile is
published when it *changes* and a stable loop's newest one can be weeks old.
Then the phone answered:

```
profile: core 1788396070077, keys none
```

**The data was missing too, and for a reason that is about the design rather
than the migration.** A profile record is emitted on a profile switch. The old
vault hid that: a reader opens the whole grant, so a profile from March is still
there in September. The keys vault is read a day or two at a time — that is the
entire point of it, and what makes a 24-hour grant possible — and those days
contained no profile at all. §2 says it plainly:

> without basal rates, ISF, IC and targets by time of day, a consumer cannot say
> what the loop was trying to do, and the insulin records become uninterpretable

That has to be true of each **day**, not of the archive. So every epoch now
carries the profile that was in force at midday of that day — the real record
with its real timestamp, through the same converter as everything else, a
re-publication rather than a synthesis. Once per epoch per process: the vault
drops what a segment already holds, but the delta would publish it again, and
once a minute is a hundred times a day of nothing.

Three readers, three gaps, one shape: **the emitter always drained everything,
and each reader was built far enough to look like it worked.**

### ✅ CLOSED — the whole reader surface, on two phones, 2026-09-13 16:50

The loop phone, one pass after the install:

```
swarm: drained 1 — cgm=1
swarm: carried the profile into epoch 20709
swarm: sealed epoch 20709, 2 records
swarm: shadow agrees — given 2, holds 1204, missing 0, lost 0, failures 0
```

Ayni, three minutes later, having had to replicate the segment first:

```
profile: core 1788396070077, keys 1788396070077
```

The same record, with the same timestamp, out of both vaults — so the target
band and the scheduled basal survive a cutover, and a percentage temp basal
still means what the pump meant by it. The loop applied a TBR forty seconds
after the install and kept looping.

All three readers now agree on real data on real hardware:

| | core | keys | |
|---|---|---|---|
| readings | 401 | 223 | merged, keys still filling in from this morning |
| treatments | 171 | 130 | every keys row identical to a core row |
| profile | `…070077` | `…070077` | the same record |

The keys columns are smaller because publishing started at 10:26 today and
nothing backfilled. That resolves itself with time, and the merge means nobody
sees the difference while it does.

### 🐛 The soak found a way for anyone to grow somebody else's phone

Set up to measure storage overnight, and the baseline itself was the finding.
In 2½ hours the keys directory grew 1.9 MB while the segments — the actual
records — grew 76 KB. The rest was a SQLite WAL and a group state file, and the
group state file is the one that matters:

```
17:00:23  swarm: handed over a reader as 265a21af855bdaf6… (follow)
17:02:11  swarm: handed over a reader as 265a21af855bdaf6… (follow)
```

**The same reader, granted again every two minutes, for ever.** A D27 handover
is a request the follower cannot tell landed, so it asked again on every pass,
and the subject's phone granted again every time. Each re-grant is a valid
control message that stays in the log for ever and a member re-added in the DGM
state that every party keeps and replays:

| | 10:26 | 17:02 |
|---|---|---|
| control operations | 0 | 68 |
| `group.cbor` | 3.0 KB | 17.6 KB |

Climbing, on a phone driving an insulin pump, driven by a message any peer can
send. That is remote-triggerable growth, not an inefficiency.

`Vault::grant` returns `Error::AlreadyGranted(tag)` now rather than adding a
member who is already there. What makes the check possible is D13 itself: the
tag is `HKDF(ECDH(subject, reader), purpose)`, so it is *stable across retries* —
the same pair and purpose always name the same member. A revoked reader is out
of the group, so letting them back in still works.

The receiving side is where it had to be fixed. The follower asks every half
hour now instead of every pass, and the plugin says "handed over a reader" once
per reader instead of every time one is offered — but throttling the sender is
politeness. **A phone must not be growable by somebody else's retry loop.**

⚠️ **Still open, watched overnight:** `keys.sqlite` is 4 KB with a 2.9 MB WAL
and has never checkpointed since the vault was created. SQLite's auto-checkpoint
fires at 1000 pages (~4 MB), which it has not reached, so this may simply be
normal and bound itself. The soak will say.

⚠️ **And a question the cutover raises:** `accept_handover` proves a reader
already holds a grant *on the core vault*. Once revocation moves to the keys
vault, revoking there and not on the core vault would leave a handover as a way
back in. Nothing to fix today — revocation is still a core-vault operation — but
it must be fixed with whatever UI takes revocation over.

### 🐛 And the door the re-grant loop was holding open

Fixing the retry loop meant looking at what a retry *does*, which is where the
worse problem was. A D27 handover proves the sender already reads this subject
**on the old vault**. That is exactly what somebody revoked on the **new** vault
still has.

So: revoke a follower on the keys vault, and their next pass hands over again
and puts them back. No scan, no prompt, nothing on screen. Revocation silently
undone for precisely the person it was aimed at — by a retry loop that is there
by design.

The group keeps a tombstone of everyone removed now, written in the DGM's
`remove` hook so no other path to removing a member can skip it, and there are
two grant doors differing on one thing only:

| | may re-add a revoked reader | reached from |
|---|---|---|
| `Vault::grant` / `keysGrant` | yes | a person scanning an invite |
| `Vault::grant_unattended` / `keysGrantUnattended` | no | a message off the network |

Not a boolean with a default: a distinction that can be got wrong by omission
will be. The refusal is logged once per reader — "refused a handover from a
revoked reader" — because it is the mechanism working, not a fault, and a line
printed every pass would become noise nobody reads.

This was written up an hour earlier as "nothing to fix today — revocation is
still a core-vault operation". That was true and it was the wrong call: the
cutover is tomorrow, the whole point of it is that revocation moves to the keys
vault, and a lock installed after the door is used is not a lock.

### ✅ Both overnight questions answered by 18:50, not by morning

**The WAL checkpointed, so storage is bounded.** The hypothesis was that SQLite
had simply not reached its 1000-page auto-checkpoint threshold. It did:

| | 17:10 | 18:50 |
|---|---|---|
| `keys.sqlite` | 4 KB | 819 KB |
| `keys.sqlite-wal` | 2,905 KB | **66 KB** |

885 KB of store against 266 KB of log content — SQLite page overhead, not a
leak. Reading the WAL as growth was wrong, and measuring it for an hour and a
half was the only way to know that.

**And `group.cbor` sat at 18,374 bytes the whole time** — 17:10 to 18:50, across
two reinstalls and a phone that went away and came back. Before the re-grant
fix it climbed about 2.2 KB an hour. That is the fix confirmed over a real
interval rather than over one pass.

**The keys vault reached parity, which is the cutover condition:**

```
18:52  merged 394 core + 394 keys = 394 for 552f688a
18:52  merged 177 core + 166 keys = 177 treatments for 552f688a
18:52  profile: core 1788396070077, keys 1788396070077
```

394 of 394 readings. At 14:43 it was 223 of 401 — the difference is eight hours
of publishing catching up with a six-hour window, exactly as predicted, with the
merge covering the gap while it did. Treatments are 166 of 177 and closing.

The one number that must stay boring is `group.cbor`, and it is.

### 🐛 Withdrawing reached the wrong vault

The grant path was fixed weeks ago: a scan grants on **both** vaults, because
"the reader gets wraps for the old vault and is not a member of the new one" is
a follower who stops reading the day the old vault goes away. Withdrawing was
never given the same treatment. It removed the core-vault member and nothing
else.

After the cutover that is the worst shape a control can have: you withdraw
somebody from the vault that no longer holds the data, the log says
`withdrew … — immediate`, and they carry on reading the vault that does. **A
safety control that silently does nothing is worse than one that is missing** —
a missing one sends you looking for another way.

Nothing had kept the keys tag, either. It came back from `keysGrant`, went into
a log line and was dropped, so even wired up there would have been nobody to
name.

D13 is what makes the fix small. The tag is `HKDF(ECDH(subject, reader),
purpose)` — derived, not assigned — so the way out can recompute exactly what
the way in created. The only thing worth storing is the identity it derives
*from*: one input, two derivations, and no second book to fall out of step with
the first. It goes in `readers.json`, the private book that already exists and
already never leaves the phone, written at the two moments it is known — a scan
that reads a v3 invite, and a handover at the instant it proves one.

```
swarm: withdrew 265a21af855bdaf6… from segment 41 — immediate
swarm: withdrew 265a21af855bdaf6… from the keys vault too
```

And when this subject has never seen a keys identity for them — an older reader,
or one whose app has no keys vault — it says so rather than passing over it,
because "no keys member" and "failed to remove the keys member" look identical
from outside and only one of them is fine.

**Three halves found in one evening, all the same shape:** the emitter always
did the whole job, and each consumer was built far enough to look like it
worked. Glucose without treatments. Treatments without the profile. Granting
without withdrawing.

⚠️ **And it heals rather than being right immediately.** A reader granted before
the book had this field has no keys identity on record, and looks identical to a
reader who never had one — except the first is still reading. So the empty case
warns rather than reassuring:

```
swarm: no keys identity on record for 265a21af… — the core withdrawal stands,
       but if they are a keys member this did NOT remove them.
       Retry once they have handed over (within 30 minutes).
```

The gap closes itself: `accept_handover` records the identity at the moment it
proves it, and a follower hands over every half hour. But "I have no record" and
"there is nothing there" are different statements and only one of them is
reassuring, so the phone says the one that is true.

### ✅ CLOSED — the withdrawal chain, on two phones, 2026-09-13 19:44

The prediction was arithmetic, and it held. Ayni's last handover was 19:11:54,
its passes are exactly two minutes apart, and the throttle is thirty minutes:

```
19:43:03  (ayni)  handed over to 552f688a
19:43:24  (aaps)  handed over a reader as 265a21af855bdaf6… (follow)
19:44             readers.json now carries a keys identity
```

And the recorded identity is **byte-identical to the one Ayni publishes** — 522
characters, compared field-for-field against the invite on its own screen:

```
book entries: 7   with a keys identity: 1
Ayni's reader key found in the book: True
  identity len : recorded 522, invite 522
  IDENTICAL    : True
```

That closes the chain without pulling a single byte of key material off either
phone. `vaultReaderKeys` will hand `keysRevokeReader` exactly the identity that
`keysGrant` was given, and `tag_of` derives from it exactly what `grant`
derived — which is the property the unit test pins.

⚠️ **One reader of seven, and that is correct rather than a shortfall.** Only a
follower whose app actually hands over can be recorded; the other six are older
pairings that may never have had a keys vault at all. They keep warning
honestly when withdrawn rather than claiming a success they cannot deliver.

*(Earlier in this file I wrote "nine readers" from a log line counting grant
events. The book holds seven; `9 grants` counts statements in `grants.ndjson`,
which includes more than one per reader.)*

### 🐛 The fourth half, found by looking instead of waiting

Three consumers had been built far enough to look like they worked, so rather
than wait for a fourth, I compared what the emitter produces against what
anything reads:

```
emitted:  bolus carb cgm event extbolus profile target tbr     (8)
consumed: bolus carb cgm       extbolus profile        tbr     (6)
```

**`target` is a temporary target, and nothing had ever read one.** The hero card
shows a band labelled as the subject's, and the code drawing it is explicit that
attributing a threshold to somebody who never published it is not allowed. But
the band came only from the profile — so a subject running an exercise target,
or eating soon, or treating a hypo, had their *profile* band displayed as theirs
while the loop was aiming somewhere else entirely.

Not a cutover regression: both vaults ignored it equally. Which is why it
survived this long.

Two things nearly went wrong writing the fix, and both were caught by checking
rather than by a test:

⚠️ **I assumed `dur` was minutes.** `TT.duration` is documented in the AAPS
source as milliseconds, and the arithmetic is far worse than the "thirty hours"
I first wrote. The one temporary target in this subject's history is 60 minutes
— 3,600,000 ms. Treating that number as minutes gives `3,600,000 × 60,000` ms,
which is **6.8 years**: the follower would have shown an expired exercise target
as running, permanently, overriding the profile band for the life of the app.

The real record settles it out loud:

```
temp target: set 8 hours ago for 60 min, expired
```

60 minutes read correctly, judged expired, profile band shown — which is exactly
what the screen says: `target 7.0–7.0 mmol/L`.

⚠️ **A cancellation is a record, not an absence.** AAPS calls a temporary target
off by writing another one with zero duration. "Newest wins" is not merely the
right rule for choosing between live targets, it is the only rule that sees a
cancellation at all — a reader taking the newest target *with a non-zero
duration* would show one somebody switched off an hour ago. There is a test.

The card says when a band is temporary and why — `target 4.4–7.8 mmol/L ·
temporary, activity, 22 min left` — because a band that silently changes reads
as somebody editing their profile rather than going for a run.

`event` is the remaining unread kind: site changes, sensor changes, notes. It is
informational rather than clinical and nothing on screen claims otherwise, so it
stays a known gap rather than a defect.

### ✅ CLOSED — the cutover, actually tested, 2026-09-13 20:16

There was no switch to throw. Every reader merges both vaults, which is right
during a migration and is also exactly why "the keys vault can stand alone" had
never been checked: **the old vault has been quietly covering for the new one at
every step.** A gap in the new one is invisible while the old one is still
there — which is the precise condition that hid three half-built readers.

So Ayni gained a second toggle, *Old vault: ignored*, which skips the core reads
rather than discarding their results afterwards. On phone B, with it on:

```
profile: core none, keys 1788396070077
```

and on screen: **12.1 mmol/L, 18 seconds old**, a full six-hour curve with no
gaps, the target band, an 8 U bolus, 80 g of carbs, the basal trace and its
`sched 0.45` line. All of it out of the keys vault, with the old one not read at
all.

That is the cutover. It is not a procedure and not a day — it is one tap, and it
is reversible with the same tap, which is what the description on it says:
*"Turn it back on if the graph looks wrong."*

⚠️ **Left ON overnight on phone B**, deliberately. A migration that only ever
runs with a safety net underneath it has not been tested; if the keys vault
falls behind while nobody is watching, the graph will show it by morning, which
is the whole point of the mode.

### 🐛 The night it went dark, and why no test could have caught it

The keys-only mode was left on overnight to find out whether the new vault could
stand alone. By morning the follower showed **"Nothing readable yet"** — and the
first useful fact was that turning the old vault back on changed nothing. Both
vaults were empty together, which says transport, not storage.

The publisher was flawless throughout: 682 operations published overnight,
`shadow agrees — missing 0`, still sealing. The follower held 288 segments and
nothing newer than 20:30 the previous evening, and `refresh_follows` reached
**nobody** — with a fresh process, on the same wifi, 192.168.88.224 and .213.

**p2panda's mDNS was running and hearing nothing, on both phones, since the
beginning.** `MdnsDiscovery` is spawned in `Active` mode, and the code carries a
comment warning that without a mode "two peers on the same wifi never see each
other". Android has a second switch underneath that one: the wifi chip does not
deliver multicast to userspace unless an app holds a `MulticastLock`, which
needs `CHANGE_WIFI_MULTICAST_STATE`. Neither app declared it. Nothing took a
lock. AAPS's `:5353` socket showed `Recv-Q 0` — the packets never arrived.

**No desktop test can show this.** A laptop has no such switch. That is why ~125
green tests and a full day of two-phone work never came near it, and why it took
a night of one phone sleeping and coming back on a different address.

It was masked because *nothing had to be discovered*. A follower can reach a
subject on the address it learned when the invite was scanned, and that works
for as long as nothing moves. The relay leg would normally cover the rest — but
the loop phone holds **no TCP connection at all**, and iroh keeps its relay as a
long-lived TCP/443. With both legs down there was no route left.

Fixed by giving the library what it needs: the permission in both manifests, and
a `MulticastLock` held for the process — not per pass, because a peer that moves
has to be findable at any hour by an app in the background, and a per-pass lock
would be absent exactly when the phone is idle.

The recovery is unambiguous. AAPS took its lock at 08:00:07; the follower had
been answering `refreshed 0` for twelve hours:

```
08:00:07  (aaps)  WifiService: acquireMulticastLock lockTag=diaswarm-mdns
08:00:31  (ayni)  refreshed 1 subject(s)
08:01:46  (ayni)  merged 411 core + 155 keys = 411
08:02:16  (ayni)  merged 411 core + 410 keys = 411
08:02:30  (ayni)  pool 3  4  1  2  0
```

Caught up to parity in two and a half minutes, and the pool found its peers for
the first time — 3 members where it had been 1 all morning. On screen: **5.9
mmol/L, 5 minutes old**, the full overnight curve, `sched 0.50`, a 4 U bolus and
35 g of carbs from half an hour earlier.

⚠️ **The relay leg is still down, and that is a separate and larger problem.**
Ping reaches the relay from both phones and the URL parses, but neither app
holds a connection to it. Everything above restores discovery *on one wifi*.
Following somebody from another network — the actual point of a relay, and
recorded as working on 2026-09-11 — is not currently working and has its own
investigation.

### 🔍 And the relay leg: it connects, then quietly stops being there

The first read of this was wrong and worth correcting. "The relay is broken" —
it is not. An endpoint built exactly as the app builds one reaches its home
relay from a laptop in under a second, with or without the trailing dot in
`DEFAULT_RELAY`:

```
CONNECTED "https://aps1-1.relay.n0.iroh.link."  state: Connected
CONNECTED "https://aps1-1.relay.n0.iroh.link"   state: Connected
```

And on the loop phone, straight after a restart:

```
ESTAB 192.168.88.224:45840  5.223.65.62:443  users:(("cout.androidaps",pid=12109))
ESTAB 192.168.88.224:52900  5.223.65.62:443  users:(("cout.androidaps",pid=12109))
```

**What is broken is that it does not stay.** Twelve hours into the previous
process — a night with the phone asleep — that same app held no TCP connection
at all. A relay cannot push to a node that is not connected to it, so a phone in
that state is unreachable from every other network, while looking perfectly
healthy from its own side: still sealing, still publishing, `shadow agrees`.

That is why both legs were down at once. mDNS was deaf permanently; the relay
went quiet overnight. Either one alone would have been invisible.

**Instrumented rather than repaired.** Every pool pass now reports it, in the
line both apps already print:

```
pool 1  1  1  0  0  relay=connected(1)
```

Re-creating an endpoint is a heavy, disruptive act and the right trigger is not
yet known — whether it drops on doze, on a wifi change, or after a fixed idle
period changes what the fix should be. That needs to be watched over days on a
phone that sleeps, and now it can be. Guessing at a reconnection policy from one
night's evidence is how the last two defects got written.

### 🔬 Don't wait a night — force the condition (2026-09-14, four minutes)

The relay question looked like it needed days of watching. It needed
`dumpsys deviceidle force-idle`:

```
08:22:23  idle=IDLE     relay_sockets=2
08:22:57  idle=IDLE     relay_sockets=0     ← 35 seconds into deep doze
08:23:10  idle=ACTIVE   relay_sockets=1     ← 13 seconds after waking
```

**Deep doze closes the relay connection, and waking restores it.** Not a leak and
not a permanent failure: while a phone is in deep doze it is unreachable through
the relay, because a relay cannot push to a node that is not connected to it.

Two things that came out of it:

⚠️ **The battery whitelist does not protect the socket.** AAPS is whitelisted on
that phone — `user,info.nightscout.androidaps,10249` — and its relay socket died
with the rest. Ayni is not whitelisted at all. Whatever the eventual fix is,
"ask for the exemption" is not it.

✅ **And with mDNS working, losing the relay no longer means losing the subject.**
Ayni's own log, during the test:

```
relay=disconnected
refreshed 1 subject
```

Reached on the LAN with no relay at all — which is exactly what the multicast
lock was for, and what was impossible yesterday.

### ⚠️ A confound in yesterday's conclusion, which I should own

I reported that the multicast lock fixed the outage. The recovery was real —
`refreshed 0` for twelve hours, then `refreshed 1` twenty-four seconds after the
loop phone's install — but that install did **two** things at once: it gave AAPS
the multicast lock *and* restarted AAPS, which re-established its relay
connection.

Three restarts of the follower alone had changed nothing, so the fix was
certainly on the publisher's side. Which of the two it was, that log cannot say.

What is independently established is that the lock was missing and necessary:
`MdnsDiscovery` runs in `Active` mode, the wifi chip does not deliver multicast
without a `MulticastLock`, p2panda cannot take one because it is an Android API,
and AAPS's `:5353` socket read `Recv-Q 0`. And the line above — `relay=disconnected`
followed by `refreshed 1 subject` — is mDNS working on its own, which is the
lock doing its job with the relay out of the picture.

### ✅ Transport gets tests, because chasing it on phones is not a method

Two transport defects in one morning, both found by hand on two phones, both
invisible to ~125 green tests. The reason they were invisible is structural, and
visible in one line of every existing test:

```rust
add_follow(&store, &subject_hex, &upstream(&addr), Some("follow"))
```

**Every test hands the follower an address**, captured from the subject's live
endpoint moments earlier. That proves fetching. It cannot prove *discovery*,
because nothing ever has to be discovered — and a follower that can only reach a
subject at the address it was handed works perfectly until the subject's phone
changes address, which happens every night.

`tests/reachability.rs` covers what was missing:

| scenario | what it guards |
|---|---|
| a node id with **no address at all** | what an invite actually carries — id + relay, never an IP |
| the subject **changes address** | the 2026-09-14 outage: same key, new socket, must be found again |
| a swarm with **no usable relay** | must say `none`, never read as connected |
| a **stale** recorded address | the real shape of it: a wrong address, not a missing one |
| the **follower** restarts | app killed overnight, must re-find from disk with no scan |
| one unreachable subject **among several** | a parent with more than one child |
| coming back from a **long outage** | must collect every missed day, not just the newest |
| a node id **nobody serves** | the guard on the positives — `reached()` must be able to be false |

They use `join_network` — isolated network id, no relay — so they run offline
and in CI, and what they exercise is local discovery. The relay leg keeps its
own guard in `bin/relaycheck`, which needs the internet and must not turn CI red
when a relay operator reboots.

**And they were checked against a mutant.** Pointing the first test at a node id
nobody is serving makes it FAIL — which is the whole difference between a test
and a decoration, and this project has shipped a verdict that compared a number
with itself before.

⚠️ **One class stays untestable here: Android itself.** No desktop has a wifi
chip that drops multicast, and none has doze. So the multicast lock is guarded
by reading the source instead — `both_apps_still_ask_for_multicast_and_take_the_lock`
asserts the permission is in both manifests, that something still calls
`createMulticastLock`, and that both endpoint entry points call it. Crude, and
the alternative is finding out on a phone again: a permission dropped in a
manifest merge looks exactly like the network being quiet. Verified it fails
when the permission is removed.

**Still not covered, and worth being honest about:** doze, wifi handover, and
the relay dropping are all real scenarios that only a phone can run. What exists
for those is instrumentation — `relay=` on every pool pass — and
`dumpsys deviceidle force-idle`, which answers in four minutes what looked like
a week of waiting.

### 🐛 And the tests immediately found one by themselves

`one_unreachable_subject_does_not_stop_the_others` failed the first time it ran.
`refresh_follows` was a sequential loop with no per-follow bound, so a subject
whose phone is flat did not fail fast — the dial waited out QUIC's own
timeouts, and every subject *after it in the file* waited with it. The flagship
is a parent with more than one child: that is a second child's readings stopping
for a reason that has nothing to do with them, with the screen saying only that
the data is old.

Fixed two ways — bounded per follow, and run concurrently.

**Then mutation testing corrected the claim.** The first version of that comment
said the test proved the concurrency. It does not:

| mutation | result |
|---|---|
| revert to a sequential loop, keep the bound | **passes** |
| keep concurrency, remove the bound | **fails** — "refresh_follows never returned" |

So the load-bearing fix is the *bound*. The parallelism is real and worth
having, and that test is not what proves it. Both the code and the comment say
so now.

That is three mutation checks on this suite — the permission guard, the bare
node id, and this — and one of them changed what the code claims about itself.
A test whose failure mode has never been observed is a decoration.

### ✅ A sleeping subject CAN be reachable — it needs a `WifiLock`

The question was a good one: if AAPS keeps BLE alive all night to receive
readings, why can a socket not stay up? Two answers.

**First, Nightscout does not solve this — it avoids it.** That is a push model:
the phone makes brief *outbound* HTTPS calls to an always-on server, and the
follower reads from the same server. Neither phone ever has to be *reachable*,
and Doze permits short outbound bursts. The always-on third party absorbs the
whole problem. This project deliberately has no such party, so the subject's
phone has to be reachable *inbound* — a connection that persists, not a burst
that succeeds. Strictly harder, and the cost of not having a server.

**Second, BLE stays up because something holds it up, and nothing was holding
the radio.** AAPS runs a foreground service with an ongoing notification; that
keeps the *process* alive. It does not keep the *wifi radio associated*. The
network equivalent is `WifiManager.createWifiLock(WIFI_MODE_FULL_HIGH_PERF)`,
and nothing in this repo took one.

Measured on phone B, same phone, same session, `dumpsys deviceidle force-idle`:

| | 20s | 35s | 1 min | 2 min | 2m40s |
|---|---|---|---|---|---|
| **lock held** | 2 sockets | estab=1 | estab=1 | estab=1 | estab=1 |
| **lock released** | — | **estab=0** | estab=0 | estab=0 | — |

Gone within seconds without it, alive throughout with it. No permission needed:
`acquireWifiLock uid=10253 lockMode=3` is granted on request.

**It is a choice in Ayni, because it spends somebody's battery.** New row in the
⋯ sheet, above the experimental ones since it is about freshness rather than
migration:

> **Keep up to date while asleep: on**
> Holds the wifi connection open so readings keep arriving with the screen off.
> Uses more battery. Off, they catch up when you next open the app.

Default on — this app exists to answer "how are they right now", and a silently
stale graph is its worst failure — and one tap gives exactly the old behaviour,
which was never broken, only late. Applied immediately rather than at next
launch, because a switch that takes effect later is a defect this app has
already had once. The lock is *released* as well as acquired, verified both
ways: `wifi lock held` / `wifi lock released — reachable only while awake`.

**Unconditional in the plugin.** That phone is the one being read: if it is
unreachable nobody sees anything, and it already holds a foreground service and
a wake lock to drive a pump. The follower gets the choice because there the cost
lands on a different person's battery than the benefit.

⚠️ **Still true: a phone on a charger never dozes at all** (`mCharging=true` →
`mState=ACTIVE`), which is why the loop phone survived being unplugged and
carried around this morning and did not survive the night. The lock is what
makes an *unplugged* phone reachable.

### 🔍 The audit I never ran, turned into a test — and the test was wrong first

Two audits had been run that week: emitted record kinds against consumed ones,
and public Rust functions against called ones. Neither pointed at the
Kotlin↔JNI seam, which is where the defect was. `swarmTick` sat in
`SwarmNative.kt` declared and called from nowhere, in a file edited six times
that day.

Running that audit now finds fourteen uncalled declarations — and that is the
problem with the naive rule, because thirteen of them are dead **by decision**:
the `spaces*` family belongs to the layer D26 dropped, and a few keys helpers
were superseded. Nothing distinguished those from the forgotten one. So they are
listed by name with a reason, and the list is the point: a new declaration that
nothing calls fails immediately.

**Then mutation testing threw the first version away.** Deleting Ayni's call to
`swarmTick` left it **green**, because the check unioned call sites across both
apps and AAPS called it all along. That union is exactly the shape of the bug —
one app does it, the other does not — so the test could never have caught the
thing it was written for.

The rule that works is per app, and narrower:

> An app that calls `swarmJoin` must also call `swarmTick` and `swarmLeave`.

Not every app should call every function — a follower has no business granting,
a publisher does not read its own vault. But every pool member owes the pool the
same three things, and a member that only joins announces nothing, learns
nobody, and can reach a subject only at the address it was handed. Mutation
confirms it now fails correctly:

```
follower/src/main/kotlin calls swarmJoin but never swarmTick.
```

Four mutation checks on this suite now, and **two** of them changed the design
rather than confirming it.

### ❌ The `WifiLock` does not work, and cannot — the record corrected

Two hours unplugged and still, sampled every two minutes. The result is
unambiguous and it falsifies the previous section:

```
09:38 → 10:05   ACTIVE        relay_estab=1   refreshing every 2 min
10:06:21                                       last fetch
10:07           IDLE_PENDING  relay_estab=0   relay=disconnected
10:09 → 10:49   IDLE          relay_estab=0   last_refresh frozen at 10:06:21
```

Forty minutes of deep doze, no relay, no fetches. The follower was completely
dark, with the lock held throughout.

**Why the forced test lied.** `dumpsys deviceidle force-idle` jumps straight to
IDLE without the sensing and standby transitions, and I watched it for two and a
half minutes. Natural doze took twenty-eight minutes to get there and killed the
socket at `IDLE_PENDING`. A four-minute forced test is a *screening* tool; it
told me the fix worked when it does not.

**And the diagnosis is exact, because the phone says so.** The lock is still
held — and it is the wrong lock, at the wrong layer:

```
Locks acquired: 0 full high perf, 2 full low latency
WifiLock{diaswarm-reachable type=4 ... nz.diaswarm.ayni}

UID=10253 blocked_state={blocked=DOZE|RESTRICTED_MODE|APP_BACKGROUND,
                         effective=DOZE|APP_BACKGROUND}
```

Two independent reasons it can never work:

1. **`WIFI_MODE_FULL_HIGH_PERF` is deprecated and was silently mapped to
   `FULL_LOW_LATENCY`** — `type=4`, and the counter reads zero high-perf locks.
   Android honours low-latency mode only while the screen is on and the app is
   in the foreground, which is precisely when it is not needed.
2. **The radio was never the problem.** Wifi stayed up throughout — adb-over-wifi
   is how these measurements were taken. It is the *app* that is denied:
   `effective=DOZE|APP_BACKGROUND`. No lock on the radio can lift a block on the
   app.

**What the block actually names is the fix.** Those two flags are cleared by two
specific things, and neither is a lock:

| flag | cleared by |
|---|---|
| `DOZE` | the battery-optimisation whitelist — user-granted, `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS` |
| `APP_BACKGROUND` | running a **foreground service** |

Ayni has neither: no service at all, and not whitelisted. AAPS has both —
`user,info.nightscout.androidaps,10310` on the doze whitelist, and a foreground
service with its ongoing notification — which is why the publisher is the half
that has generally stayed reachable, and why the follower is the half that went
dark.

So a follower that must be current while asleep needs an ongoing notification
and a permission prompt. That is a product decision, not a bug fix, and it is
the same bargain every always-on Android app makes. The alternative is to accept
the gap and make staleness loud — the pool already covers it whenever any other
holder is awake.

### ✅ The proper fix: clear the two flags the phone actually named

The `WifiLock` is gone — it cost battery and did nothing, and so has the AAPS
preference that switched it, which controlled nothing on a phone that already
had both exemptions.

`dumpsys netpolicy` named the remedies precisely, and there are exactly two:

| flag | cleared by | who can do it |
|---|---|---|
| `APP_BACKGROUND` | a foreground service | the app |
| `DOZE` | the battery-optimisation exemption | **only the user** |

**`StayAwake`** is the first half: a foreground service that runs the sync loop
itself on the same two-minute cadence rather than leaning on WorkManager — a
periodic worker is exactly what Doze defers, so keeping a service alive so a
deferred worker can run later would be theatre.

**The second half cannot be taken, only granted**, and the row says so instead
of claiming to be finished. Three states, three sentences, all before the tap:

> **off** — "readings stop while the screen is off and catch up when you open
> the app — measured at 40 minutes behind after two hours face down. On, ayni
> keeps fetching, shows a permanent notification, and uses more battery."
>
> **on, exempt** — "Fetching while the screen is off. There is a permanent
> notification while this is on; swipe it away by turning this off."
>
> **on, not exempt** — "Fetching while the screen is off, with a permanent
> notification. If it still falls behind overnight, tap here and set ayni to
> Unrestricted — that is Android's last word on it, and only you can grant it."

⚠️ **That third string started out as a warning and had to be corrected within
the hour.** It asserted "Android will still pause ayni", and measurement
disagreed: with the service running, the phone reports

```
deep doze:  blocked=DOZE|RESTRICTED_MODE|APP_BACKGROUND
            allowed=FOREGROUND|...
            effective=NONE
```

`DOZE` is in the blocked set and `allowed=FOREGROUND` overrides it. The fetches
kept landing on their two-minute cadence throughout — `last_refresh` advanced
from `11:09:10` to `11:11:11` while the device was in deep IDLE. **The
foreground service alone appears to be sufficient, and the battery exemption
unnecessary.**

So the exemption is now offered as a remedy if it is ever needed, rather than
demanded on a theory. Asking for a battery exemption nobody needs is how apps
train people to grant them without reading.

Two settings that are easy to confuse, and the first one is not the one:

| Android setting | means | clears `DOZE`? |
|---|---|---|
| **Allow background usage** | the app is not *Restricted* — it is in the default *Optimised* bucket | no |
| **Unrestricted** | the battery-optimisation exemption | yes |

Tapping in that third state opens Android's battery-optimisation list rather
than firing `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`. The direct prompt is one tap
faster and is the one F-Droid and Play treat as a red flag, because it is the
one apps abuse. Slower, and it asks for nothing it has not already explained.

**Default off, which is a change of mind with a measurement behind it.** It was
on when the mechanism was a lock that did nothing and cost only battery. What
works costs a permanent notification, and switching that on for everybody who
updates — unasked, unexplained — is not a default to choose on somebody's
behalf.

Verified on phone B: `isForeground=true`, `stay-awake service started`, not
doze-exempt, and the row in amber saying exactly that.

### ⚠️ Everything measured today was on one LAN

Worth writing down before it is mistaken for more than it is. Phone B reports
`relay=connected(1)` while `ss -tan` shows **no relay socket at all** — because
both phones are on the same wifi and connect directly. The relay is not carrying
this traffic and has not been carrying it all morning.

So today establishes same-LAN behaviour. It says nothing new about following
somebody from another network, which has not been exercised since 2026-09-11 and
is the case a relay exists for. The overnight outage was also same-LAN: mDNS deaf
and the app doze-blocked, with the relay incidental.

**And two of the three instruments built today lie in some conditions:**

| signal | trust |
|---|---|
| `relay_estab` (socket count) | **low** — wrong twice: pinned to IPv4 while the link was IPv6, then reading 0 when there is legitimately no relay socket |
| `ayni_relay` (iroh's own state) | medium — says `connected(1)` with no socket visible |
| `last_refresh` (did a fetch land) | **high** — measures the outcome, not a proxy for it |

Read `last_refresh`. It is the only one that answers the question anybody
actually has, which is whether the readings are arriving.

### ✅ CLOSED — a sleeping follower stays current, 2026-09-14

Natural doze this time: unplugged, screen off, untouched. Fifty-one minutes in
deep IDLE without missing a fetch.

```
11:27  IDLE_PENDING  relay=connected(1)  last_refresh 11:27:05
11:29  IDLE          relay=connected(1)  last_refresh 11:29:06
 ...                 every two minutes, no gap
12:18  IDLE          relay=connected(1)  last_refresh 12:17:24
```

Twenty-six consecutive samples on the exact cadence. Against the same test on
the WifiLock build:

| | deep IDLE | fetches |
|---|---|---|
| `WifiLock` | 40 min | **none** — frozen at 10:06:21 |
| foreground service | 51 min | **26 of 26**, on cadence |

**The foreground service alone is sufficient. No battery exemption is needed** —
not "Allow background usage", not "Unrestricted". The offer of the exemption
stays in the row as a fallback for a phone that behaves differently, but asking
for it by default would have been asking for a permission that does nothing.

⚠️ **Three attempts failed before this one, all for the same reason: the phone
was still on USB power**, and a powered phone never dozes. Twice that was not
checked before walking away from the test. The rig now reports every
precondition — service running, adb-wifi up, sampler alive, log buffer, screen
off, and `USB powered` — in one command, and the answer is known in five seconds
rather than forty-five minutes.

And the run before that would have measured nothing for a different reason: the
service was dead. `StayAwake` only started from `MainActivity.onCreate`, so an
app update left the setting switched on and doing nothing until somebody opened
the app. `Restart` now puts it back on `BOOT_COMPLETED` and
`MY_PACKAGE_REPLACED`, proven by installing an update and watching it come back
without the app being opened:

```
restart after android.intent.action.MY_PACKAGE_REPLACED — reapplying stay-awake
stay-awake service started
```

"On" has to mean on.

### ✅ CLOSED — off-LAN, the case the relay exists for, 2026-09-14

The last untested one, and the one that matters for anybody who does not live
with the person they follow. The loop phone's wifi was turned off — with the
restore running *on the phone* so it could not be stranded — putting it on Spark
NZ mobile data behind CGNAT while the follower stayed on home wifi:

```
loop phone:  10.241.59.239   (CGNAT, no route from the LAN)
follower:    192.168.88.213  (home wifi)

12:23:26  (ayni) refreshed 1 subject(s)
12:23:38  (ayni) refreshed 1 subject(s)
12:25:27  (ayni) refreshed 1 subject(s)
```

Three fetches across two networks with no LAN path between them, so the relay
carried every one. Sustained on cadence rather than a single lucky connection.

That completes the set. Everything unknown at the start of the day now has a
measurement behind it:

| | |
|---|---|
| same wifi, both awake | ✅ |
| follower asleep, unplugged | ✅ 51 min deep doze, 26/26 fetches |
| subject changes address | ✅ test suite and hardware |
| **different networks, via relay** | ✅ |
| service survives update and reboot | ✅ |
| one unreachable subject among several | ✅ bounded, does not block the rest |

⚠️ Still not done: a real overnight rather than an hour, and the `event` record
kind, which nothing reads and nothing on screen claims to.

### 🐛 The off-LAN "proof" tested the wrong path, and hid a stalled subscription

Twenty minutes after declaring transport closed, the follower was showing a
reading **nine minutes old**, then ten, with the row count sliding *backwards*
(377 → 376 → 375) as the six-hour window moved on with nothing new arriving.

**What I actually proved off-LAN was the core-vault fetch.** `refreshed 1
subject(s)` is `netRefresh` — the `diaswarm-net` path. Phone B is in keys-only
mode, so what it *displays* comes from the keys vault, replicated by p2panda log
sync, which is a different mechanism over a different subscription. The relay
carried core fetches across networks perfectly. Keys replication stopped at the
same moment, and I called the whole thing closed on one of the two.

**The defect underneath is real and is the same shape as `swarmTick`.** After
the publisher changed network, the follower's log-sync subscription wedged and
never recovered:

```
12:24  last segment received
12:33  keys # opened 640 rows 377   "9 min ago"
12:34  keys # opened 640 rows 375   "10 min ago"
       keys carrying 2 log(s)        ← printed every pass, the whole time
```

`keys carrying 2` every pass, so the app believed it was subscribed. Eleven
minutes, no self-healing, and the publisher was back on its original address the
whole time — so not an address problem.

A force-stop and relaunch fixed it in one pass: `66s ago`. The data had been
sitting on the publisher all along.

⚠️ **So "carrying N logs" is not evidence of anything.** It reports that a
subscription object exists, not that bytes are moving — which is precisely the
distinction this project keeps being caught by. The honest signal is the same
one as everywhere else: did a record actually arrive.

### 🔭 And the signal that would have caught it a minute in

The stall was found by somebody glancing at a phone. Nothing in the app noticed,
because nothing in the app was measuring the right thing — `keys carrying 2
log(s)` reports that a subscription object exists, and printed faithfully
throughout.

So the pass asks the question the hero card asks, once a pass:

```
keys newest for 552f688a: 188s old
keys newest for 552f688a: 134s old
```

Falling, so replication is healthy. Rising past ten minutes is a stall — a CGM
produces a reading a minute, so ten minutes of silence is not a slow network —
and the remedy is already known, because a force-stop fixed it in one pass.
`Endpoint.restart` does the same without the user being involved.

Two judgement calls worth stating:

- **Ten minutes** before calling it. Being wrong costs a reconnect; being eager
  costs one every pass.
- **Fifteen minutes minimum between reconnects**, persisted across process
  restarts, because a rate limit that forgets itself on restart is not a rate
  limit. A subject whose phone is simply off produces *identical* silence, and
  reconnecting every two minutes for somebody asleep would be this app making
  its own weather.

⚠️ **The trigger has not fired in anger.** Detection is proven and the remedy is
proven; the condition joining them has not yet been met, because nothing has
been stale for ten minutes since it was written. Manufacturing a stall means
toggling the network on the phone that drives a pump, which is not worth it —
the next real one will exercise it, and now it is visible either way.

**Fourth time today a proxy lied and the outcome measure told the truth:** the
socket count wrong twice, `relay=connected(1)` with no socket, `keys carrying 2`
for eleven silent minutes. The only signal that has never misled is whether a
record actually arrived.

### ⚠️ Three diagnoses in an hour, each from four samples

Worth recording as a process failure, because the code is fine and the method
was not.

| reading | evidence | verdict |
|---|---|---|
| "subscription wedged permanently" | 11 min stale, restart fixed it | **wrong** — it recovers on its own |
| "healthy sawtooth" | 188 → 134 → 185 | **wrong** — the next sample was 305 |
| "intermittent, gaps to 7 min" | 305 → 426 → 426 → 126 → 246 | plausible, still only five samples |

The first one produced a real fix (`restream_if_quiet`, a targeted re-subscribe
instead of restarting the whole endpoint) and a real threshold (ten minutes)
chosen on the assumption that a stall is permanent. If gaps of seven minutes are
*normal*, that threshold is one bad minute away from reconnecting needlessly for
ever.

The honest position: **the publisher seals every minute and the follower sees
data in bursts several minutes apart, and I do not yet know why.** It could be
gossip batching, a sync session triggered by something periodic, or the pass
cadence interacting with the stream. Each implies a different fix.

So: `agewatch.sh` records every distinct freshness reading to `age.csv`, and the
threshold gets chosen from the distribution of real gaps rather than from the
largest number seen in a five-minute window. `restream_if_quiet` is built and
tested and deliberately **not installed** — installing restarts the app, which
resets exactly the measurement being taken.

The freshness probe earned its place regardless: none of this was visible at all
until the pass started asking whether a record had actually arrived.

### 🔍 The mechanism, finally readable: live mode delivers nothing

The replicator has recorded sync events since it was written, with a comment
saying why — "nothing replicated" has several very different causes — and **no
JNI ever exposed them**. So every explanation of a stall on a phone had been
inferred from outside, which is how three contradictory diagnoses came out of one
afternoon. Exposed now, the first read says it plainly:

```
sync: SyncFinished { received_sync_operations: 6, received_live_operations: 0 } from 9eeeac47
sync: LiveModeStarted from 9eeeac47
sync: Failed { error: "...ConnectionLost(TimedOut)" } from b8e0c9ba
```

**Catch-up sync delivers; live mode delivers nothing.** Six operations in the
session, zero live. `stream(topic, true)` is documented in this repo as "catch up
first, then keep receiving over gossip — this is what removes the two-minute
follower poll". The first half works. The second half has not been observed
delivering a single operation.

That explains the shape exactly: data arrives in a burst when a session starts
and nothing in between, so the follower's freshness sawtooths over several
minutes rather than tracking a CGM that reports every minute.

It also means the earlier readings were all wrong in an interesting way:

| said | actually |
|---|---|
| subscription wedged permanently | never wedged; sessions still happen |
| healthy sawtooth | a sawtooth, but of catch-up sessions, not live delivery |
| intermittent with ~7 min gaps | the gap IS the interval between sync sessions |

And it makes `restream_if_quiet` a symptom treatment that happens to work —
re-subscribing forces a fresh catch-up, which is the thing that actually
delivers. Worth keeping as a safety net, not worth mistaking for the fix.

⚠️ **The real question is why live mode is silent**, and that is not answered
yet: gossip mesh never forming, the topic not matching between peers, or the
`ConnectionLost(TimedOut)` above being the normal state rather than an anomaly.
That is the next thing to look at, and now it can be looked at rather than
guessed at.
