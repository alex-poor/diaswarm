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
