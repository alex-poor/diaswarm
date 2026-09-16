# Moving onto p2panda: what is proven, and what a cutover still needs

The vault, the grant model and replication have been rebuilt on p2panda's own
layers — [D20](decisions.md) and [D21](decisions.md). None of it ships. The old
implementation is still what runs on a phone, and this is the account of what
would have to be true before that changes.

It exists because "it works" is not a decision. The evidence below is specific
about what was measured, on what, and what each measurement does *not* cover.

---

## What is replaced, if this lands

> **THIS TABLE NAMED `p2panda-spaces` UNTIL 2026-09-17, AND D26 REJECTED SPACES
> ON 2026-09-12.** Five days in which the document a cutover is read off named
> the wrong destination. It is the second time — `1bdcffd` left a gate list
> behind a rewritten finding — so the rule is now explicit: a decision that
> changes direction updates this file in the same change, not in a later tidy.

| Today | Then |
|---|---|
| `diaswarm-core::vault` + `seal` — hand-composed cryptography, ~312 lines of construction in `seal.rs` under 1,310 lines of vault bookkeeping | `diaswarm-keys` on `p2panda-encryption`: `EncryptionGroup`, `SecretBundle`, `encrypt_data`/`decrypt_data` |
| per-reader key wraps, hash-chained grant log, unlinkable tags | the library's key layer, with diaswarm's segments kept |
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

## The migration path, and why there will not be one

**Decided 2026-09-17.** `ShadowSpacesVault` now defaults **on**, so every phone
writes the `diaswarm-keys` vault from the day it is installed. The point is not
shadowing; it is that a phone which has been writing both since first run has
nothing to migrate when D26 lands.

There are no real vaults yet — everything sealed the old way belongs to a
development phone — so the window in which this is free is open now and closes
the moment anyone outside development seals history. Taken now for that reason.

What it does NOT do is cut over. The old vault stays authoritative and every
screen still reads it. Promoting `diaswarm-keys` is a separate change with its
own cost, because it seals on a five-minute cadence where the old vault seals
every pass; making it authoritative unchanged would make a follower up to five
minutes staler, which is the fault §12.3 exists to prevent.

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

**4. ~~There is no migration path for an existing vault.~~ NOT REQUIRED, and
this is a scope decision rather than a solved problem — 2026-09-17.** There are
no real vaults yet; everything sealed the old way belongs to development phones.
The cutover therefore ships as the only behaviour, so that every user starts on
the upstream vault and no migration path ever has to exist. The re-drain button
covers the two development phones.

**This is only true while it is true.** The moment anyone outside development is
sealing history, this item comes back and comes back harder, because by then it
is somebody's only copy.

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

Confirmed across every session on the phone, not just the one first seen:

```
received_sync_operations: 0    received_live_operations: 0
received_sync_operations: 6    received_live_operations: 0
received_sync_operations: 0    received_live_operations: 0
received_sync_operations: 2    received_live_operations: 0
```

Sync counts vary; **live is zero every time.** And the sessions repeat —
`SyncStarted → SyncFinished → LiveModeStarted`, with the freshness age
collapsing at each `SyncFinished` and climbing until the next one. So the
follower's data arrives entirely through reconnection, and the gaps are the
intervals between reconnections.

The `Failed { ConnectionLost(TimedOut) }` fits that: connections do not survive,
each re-establishment triggers a fresh catch-up, and live mode never gets the
chance to deliver — or is not working at all. Those are different problems with
different fixes and the logs so far cannot separate them.

⚠️ **The real question is why live mode is silent**, and it is not answered yet:
the gossip mesh never forming, a topic mismatch between peers, or connections
dropping before live can carry anything. Now readable rather than guessable,
which is the whole difference.

### 🛡️ A diagnostic nothing can read is not a diagnostic

The most expensive lesson of the day, made automatic so it is not left to
anybody's judgement.

The replicator recorded every sync event from the day it was written, with a
comment explaining why — "nothing replicated" has several very different
causes — and no JNI ever exposed it. So replication on a phone could only be
reasoned about from outside, and one afternoon produced **three contradictory
diagnoses from four samples each** before anyone looked at what the code already
knew. `swarmTick` and `verify_control_chain` were the same pattern in other
layers, found the same way: by accident, expensively.

`diagnostics_can_be_read_from_a_phone` fails if a function whose name promises
to report state — `report`, `status`, `events`, `metrics`, `health`, `verify`,
`heard`, `seen` — cannot be reached from the JNI. Anything genuinely internal
goes on a list with a reason.

It caught two things immediately:

- **`holders_heard`** — who else in the pool has this subject. D15's whole
  promise is that any holder serves identical bytes, so a subject whose phone is
  asleep can still be read from somebody else; whether anybody else was there
  had never been visible. When a follower sat in a pool of one all morning, this
  is the line that would have said so.
- **its own blind spot** — it reported `verify_control_chain` unreachable
  because it only matched `.name(` and the JNI calls it as a free function. A
  guard that cries wolf gets silenced by an entry on the exemption list, which
  is how a guard stops guarding, so the check was fixed rather than the list.

Mutation-checked against the defect that motivated it: hiding the sync event log
makes it fail with `these report state and no app can read them: ["events"]`.

It is a naming heuristic and it will miss things. It is still better than what
happened today, which was noticing on the fourth guess.

### 🔍 And the first pass said nobody is holding the subject

`holders_heard` was wired up and immediately answered a question nobody had been
able to ask:

```
holders for 552f688a: none — only the subject can serve this
```

**⚠️ AND THAT READING WAS WRONG, corrected the same afternoon.** It was one
sample, taken seconds after an app restart, before gossip had delivered a single
announcement. Over a longer window:

```
 20 x  holders for 552f688a: 2      (9eeeac47, b8e0c9ba)
  8 x  holders for 552f688a: 1
  7 x  holders for 552f688a: none
```

**D15's redundancy is operational.** Two peers hold the subject most of the
time, and "none" is the state for the first minute after a restart while gossip
catches up. Declaring a structural failure from the first reading of a brand new
instrument was the fourth confident wrong answer drawn from a handful of samples
in one afternoon.

What it did usefully establish is *who*: `b8e0c9ba` is a holder, and it is also
the peer every one of the 29 `ConnectionLost(TimedOut)` failures is with.

Part of it is a choice made this morning: the follower ticks with `maxAdopt = 0`
— "the defect is discovery; whether a follower's phone should hold strangers'
ciphertext is a separate decision and does not ride in on a bug fix". That
reasoning still stands, and this is the other side of the bargain: a pool where
nobody adopts is a pool that provides nothing when it is needed.

⚠️ **Two structural findings in one afternoon, both from diagnostics that
existed and could not be read:** live mode has never delivered an operation, and
nothing holds the subject. Neither is a bug in the sense of a wrong line of
code. Both are the system not doing what the design says it does, and both were
invisible from a phone.

### ✅ Two peers, two processes, real sockets — and what it does NOT catch

Every test in this crate ran both peers in one process, which makes gossip
trivially local: a `Holding` announcement never crosses a network and live
delivery is a function call away. So a thorough pool test passes — and it is a
good test, it proves the mechanism — while two phones report `holders: none` and
`received_live_operations: 0` in every session.

`poolpeer` is one peer with a process boundary around it, printing JSON state a
second at a time. `tests/two_process.rs` spawns two and asserts three things
over real sockets:

| | |
|---|---|
| two processes see each other in the pool | the floor — below this nothing means anything |
| a `Holding` announcement **crosses a process boundary** | gossip, the same mechanism that carries live operations |
| a second process **adopts without being told to** | D15's redundancy, previously only ever asserted with `adopt` called by hand |

Twelve seconds, and mutation-checked: putting the second peer on a different
network id makes the gossip test fail.

⚠️ **And they all pass, which does not reproduce today's failure.** Gossip works
perfectly between two processes on one machine. So the gap is narrower than
"in-process versus cross-process" — it is **one machine versus two Android
devices**. Something about the phones, not the code path, is stopping gossip:
discovery, connection stability, NAT between two wifi clients, or Android
throttling a background socket.

That is worth being exact about rather than claiming the harness solved it. What
it does do is close a real hole permanently — a regression in gossip or adoption
logic now fails in twelve seconds instead of surfacing as a stale graph — and it
narrows the remaining question to something specific and physical.

### ✅ What is actually happening, measured at one instant

After three wrong diagnoses, the numbers taken from both phones at the same
moment:

```
publisher segment ops: 929
follower opened:       924        behind: 5
```

**Replication works.** The follower tracks about five operations — five
minutes — behind the publisher, converging: `873 874 878 884 886 893 895 896
904 911 913 924`. The "BG from 9 minutes ago" was a stall in progress that had
not yet reached the ten-minute threshold.

The shape of it, now fully evidenced:

| | |
|---|---|
| `LiveModeStarted` | **69 times**, `received_live_operations: 0` every time |
| `SyncFinished` | 54 — every byte the follower has came from these |
| `Failed` | 29, **all** with one peer, `ConnectionLost(TimedOut)` |
| steady state | ~5 operations behind, occasional 10–17 min stalls |
| stalls | self-heal — the detector fires and the endpoint restart recovers |

And **the targeted remedy never fired once**. `restream_if_quiet` compared its
quiet period against `last_event` — the time since *any* operation arrived — and
during a stall catch-up syncs keep trickling operations in while the newest
*record* ages past a thousand seconds. So it returned 0 every time:

```
keys stalled for 1025s and re-subscribing gave 0 — reconnecting
keys stalled for  849s and re-subscribing gave -4 — reconnecting
```

The endpoint restart, the fallback, is what has been doing all the recovering.

Fixed by moving the decision to whoever can make it: `restream()` is
unconditional now, and the app — which knows how old the newest record is —
decides when. A component that cannot know how often a subject publishes has no
business deciding what "quiet" means.

⚠️ **Still open, and now precisely stated:** live mode has started 69 times and
delivered nothing, and all 29 connection failures are with a single peer. Those
are the two things worth chasing, and both are now visible from a phone.

### ✅ The connection failures are the test rig, not the product

`b8e0c9ba` turned out to be **phone B's own AAPS** — full id in its own log,
`in the pool as b8e0c9badf6b3be3…`. So every sync failure was Ayni trying to
reach the AAPS instance on the same handset. Counted by peer:

| peer | SyncFinished | Failed |
|---|---|---|
| `9eeeac47` — loop phone, **different device** | **62** | **0** |
| `b8e0c9ba` — AAPS on the **same handset** | 15 | **70** |

**Cross-device syncing never failed once.** Same-device peering fails four times
out of five — two apps on one phone, each with its own iroh endpoint, trying to
reach each other through the wifi interface.

That is a property of this two-app test rig and not of the product: a follower
and a publisher do not normally share a handset. It is worth knowing because
those failures had been read, by me, as evidence of general connection
instability, and they are nothing of the sort.

⚠️ **Which leaves exactly one open question**, now clean of that noise: live mode
has started 69 times against a peer it syncs with perfectly and delivered
`received_live_operations: 0` every time. Catch-up works; gossip-borne live
delivery does not. That is the last unexplained thing, and it is the one that
would take the follower from minutes-behind to seconds-behind.

### ✅ Answered: nothing ever called `SyncHandle::publish`

Live mode had started 69 times and delivered nothing because **the sending half
of it was never wired**, in any build this project has ever shipped.

p2panda's `LogSync` catches a peer up and then, in its own documentation,
"nodes switch to live-mode to directly push new messages to the network using a
gossip protocol". The pushing is `SyncHandle::publish(operation)`. Three
independent lines agreed:

1. `SyncHandle::publish` is the only way to push, and it is on the handle;
2. `grep -rn "\.publish(" crates/diaswarm-net/src crates/diaswarm-keys/src`
   returned nothing — it had never been called;
3. `crates/diaswarm-net/src/replicate.rs` moved the handle into its spawned
   task as `let _keep = handle;`, where it kept the subscription alive and was
   unreachable for ever after.

So a subject wrote each operation to its SQLite store, and the store is not on
the network. Every byte any follower has ever displayed arrived in a **catch-up
sync** — which is why freshness tracked the sync interval rather than the
publish, and why an hour of tuning doze, Wi-Fi locks, the relay and the
subscription lifetime moved the number around without ever fixing it.

**What it cost:** three wrong diagnoses of one stall, a falsified `WifiLock`
change, a foreground service justified partly on the wrong grounds, and a
follower that was structurally minutes behind and looked healthy from every
angle the code could see.

**Why nothing caught it.** One counter. `received` counted operations from both
phases, and catch-up delivered everything, so the total was always healthy. The
two phases are now counted separately — `live_received()` — and that number is
zero if and only if the push half is broken.

**The fix**, in three parts:

| where | what |
|---|---|
| `replicate.rs` | the handle is kept in `handles`, not buried in the task; `broadcast(subject, op)` calls `publish` on every topic `carry` recorded for that subject |
| `diaswarm-android/src/lib.rs` | `KeysVault` holds a clone of the pool's replicator; every `wire::publish`/`publish_control` site pushes what it just wrote |
| both apps | `keysSealChecked` reports `pushed=N`; `keysSyncEvents` leads with `counts received=N live=M` |

**Tests** — `crates/diaswarm-net/tests/live_mode.rs`, three of them: a segment
sealed while a peer is already listening arrives pushed; so does a grant (the
one that strands a new reader at pairing time); and broadcasting a subject
nobody carries is 0 rather than a number that always looks healthy.

Mutation-checked. Removing the `publish` call makes the first two fail with the
exact hardware symptom in the event log — `LiveModeStarted`, then
`received_live_operations: 0`, and the operation not delivered in 45 s.

⚠️ **Not yet confirmed on hardware.** The counter to watch is `live` in the
follower's `sync: counts received=… live=…` line, and `pushed=` in the
publisher's shadow-pass line. Both are zero in every build before this one.

### ⚠️ And the live counter was wrong within twenty minutes of shipping

The first version of `live` asked `metrics.received_live_operations > 0`. That
is a fact about the **session**, not about the operation in hand: `Metrics` is
cumulative, so once a session has taken one push, every later catch-up
operation on that same session looks pushed too. Sessions do re-sync — the
phone's own event log shows `SyncFinished, LiveModeStarted, SyncFinished,
LiveModeStarted` against one peer.

Caught by two numbers that could not both be true:

| side | reported |
|---|---|
| Ayni (receiving) | `counts received=1059 live=1052` |
| AAPS (publishing, its only peer) | `shadow agrees — given 1, … pushed 1` |

The true figure was **one**. A counter added to detect a broken transport was
reporting success by accident — the same failure shape as the defect it exists
to catch, which is why it is worth saying out loud rather than quietly fixing.

`FromSync` carries a `session_id`, so the rule is now the per-session
*increase*, in `count_live`, with three unit tests: catch-up after a push is not
a push; two sessions do not borrow each other's counts; and a recycled session
id does not lose its pushes (saturating to zero there would undercount silently
for the life of the session — a working transport reported as a broken one).

### ✅ The transport suite was flaky, which is worse than absent

Two consecutive parallel runs of `tests/two_process.rs` failed two
**different** pre-existing tests; the same suite run serially passed 4/4 twice,
in 22 s and 26 s. Four tests at once is eight peers on one machine competing
for discovery.

A transport suite that flakes teaches you to discount a red result, which is
precisely how a real transport defect survived for the life of this project.
The tests now take a static lock so plain `cargo test` is honest — a guarantee
nobody has to remember is the only kind that holds.

### 📋 Where hardware verification actually stands

| claim | evidence | status |
|---|---|---|
| `SyncHandle::publish` is now called | `tests/live_mode.rs` ×4, `two_process.rs` ×1, all mutation-checked | ✅ |
| the send path runs on a phone | phone B's AAPS: `shadow agrees — … pushed 1` | ✅ |
| a push is *delivered* to a follower | — | ⚠️ **not yet** |

The last row cannot be measured on the current rig. Ayni's keys replicator
carries only *its own* subject and the subjects it *follows* — pool adoption
applies to the core vault, not the keys vault — so the only peer that can push
to Ayni is the loop phone, which is still on the pre-fix build. With the
corrected counter Ayni reports `counts received=4 live=0`, which is the right
answer and a clean negative control.

`swarm: keys subject <hex>` is now logged at plugin startup. The keys subject is
a different key from the core vault's and could not be read from a phone at
all, which is what made a laptop-side check impossible without driving the UI.

### ⚠️ And the live counter was wrong a second time, in arithmetic

Keying the per-session baseline on `session_id` alone was not enough. A
replicator streams a topic per subject, each topic manager numbers its own
sessions, so two topics run sessions numbered alike. Interleaved, every switch
between them reads as a session starting over — and counts the whole running
total again.

The phone said so in a way no interpretation survives:

```
sync: counts received=1107 live=1977
sync: counts received=8103 live=17128
```

More pushed arrivals than arrivals. Not merely wrong: impossible.

Three changes, in increasing order of how much they matter:

1. the baseline is keyed on `(topic, peer, session_id)`, not the id alone;
2. `live` is incremented in the same branch that stores the operation, so
   `live <= received` is structural rather than something to remember — and
   both the in-process and two-process tests now assert it;
3. the baseline map is bounded, and **overflows towards undercounting**.

(3) is the one worth arguing about. Refusing new baselines when full reports
fewer pushes than happened, which sends somebody to look. Clearing the map
instead would report the next session's whole running total as new — which
**hides** a broken transport. This counter has now over-reported twice; the
failure direction is not a detail.

**Three wrong versions of one counter is itself the finding.** The reason is
the same each time: cumulative metrics describe a session, and the question
being asked is about an operation. It was believed twice because nothing
asserted the one thing that cannot be true.

### ⚠️ …and a third time, which is the actual finding

The previous entry claimed `live <= received` was now structural. It was not.
`count_live` returned a **delta**, and its "the session started over" branch
returned the whole cumulative value — so a baseline going missing mid-flight
added hundreds in a single event. Tying the increment to the store branch
bounded *when* it happened, not *how much*. The phone, on the build carrying
that fix:

```
sync: counts received=3322 live=5245
sync: counts received=4737 live=6685
```

**Four versions of one counter, each wrong differently, all wrong the same
way.** Every one tried to reconstruct a per-operation fact from a counter that
describes a session, which means guessing where a session began — and every
guess had a case where it added a running total at once.

There was never a delta to compute. p2panda bumps
`received_live_operations` by exactly one immediately before emitting the
event, so the counter *changing* is the whole signal and its magnitude is
noise. `is_live` returns a **boolean**. One event is one operation, at most one
increment, and `live <= received` holds by construction instead of by argument.

| version | rule | reported |
|---|---|---|
| 1 | `received_live > 0` | 1,052 live of 1,059 |
| 2 | delta, keyed on `session_id` | 17,128 of 8,103 |
| 3 | delta, keyed on `(topic, peer, session)` | 5,245 of 3,322 |
| 4 | **boolean: did the counter change?** | — |

The lesson is not about p2panda. It is that a diagnostic whose failure mode is
over-reporting will be believed, and this one was believed three times.

### ⚠️ De-duplication means a push can be invisible, and that is fine

A laptop peer carrying phone B's publisher recorded `received=488, live=0`
while that publisher reported `pushed 1` on every pass. Not a failure of the
push: p2panda drops a live operation whose hash is already in the session's
dedup buffer, so an operation a catch-up sync has already delivered never
reaches the live counter. With sessions re-syncing every few seconds on a quiet
LAN, catch-up keeps winning the race.

So `live` measures *pushes that beat catch-up*, not pushes sent. The publishing
side's `pushed=` is the honest measure of the send half; `live` is the measure
of whether pushing is buying anything.

### ⚠️ The keys vault has no pool redundancy

`keysCarryAll` carries a phone's **own** subject and the ones it **follows**,
and nothing else — `keysCarry`, the single-subject entry point, is in the JNI
guard's known-dead list as superseded.

So D15's promise — any holder serves identical bytes, so a subject whose phone
is asleep is still readable — holds for the core vault and **not** for the
vault this migration is cutting over to. Pool adoption announces and carries
core-vault subjects; the keys logs of a stranger are never carried by anyone.

Not a defect in the live-push work, and not fixed here: it is a decision about
what the cutover means, and it belongs to whoever is making it.

> ✅ **CLOSED 2026-09-14 by [D28](decisions.md).** Bucket announcements and pool
> adoption now cover keys subjects: `BucketMessage::HoldingKeys`,
> `Swarm::keys_wanted`, and a `keysCarryAll` that adopts up to four heard
> strangers a pass and then reports what it holds so the next tick announces it.
> `tests/keys_pool.rs` has a stranger carrying a subject it was never introduced
> to, failing to read it, and re-announcing it — mutation-checked both ways.
>
> **The announcing half is not separable from the adopting half**, and D28 is
> mostly about why: a peer that says "I hold keys-subject Y" while only
> followers hold Y has published the follower set, which is the D18 leak.
>
> Two peers in one process. Not phones, and not disjoint shares — the same gap
> the core vault's pool has always had. What a phone can now say is
> `keys holders for <subject>: N`, through the new `keysHoldersHeard`; it was
> "none" by construction before, and nothing reported even that.

### ✅ First live delivery ever recorded, and it is p2panda's number, not mine

The evidence that matters does not come from the counter I wrote — which has
been wrong four times — but from p2panda's own `Metrics`, read out of the
follower's session events on phone B. Counting every session by peer:

| peer | build | sessions | `received_live_operations` |
|---|---|---|---|
| `9eeeac47` — the loop phone | **pre-fix** | 175 | `0` in every one |
| `b8e0c9ba` — phone B's AAPS | **fixed** | 55 | `0` |
| `b8e0c9ba` — phone B's AAPS | **fixed** | 1 | **`7`** |
| `073e07e9` — laptop watcher | fixed | 16 | `0` |

One non-zero, from the one peer running the fix. The publisher's own log for
that window says `pushed 1` on each of four passes, plus its group-create and
grants — which is where seven comes from.

**Before this change that column was zero in all sixty-nine sessions ever
observed.** It is now non-zero, from the peer that publishes, and still zero
from the peer that has not been updated. That is the control and the treatment
in one table.

⚠️ **One number is discarded rather than explained.** A laptop peer reported
`live=899` against a publisher that had pushed 4. It reconciles with neither
the publisher nor the phone follower, and that peer is not like the others —
it carries the subject at four depths at once and receives other subjects'
logs over the same topics. Until it is accounted for it is not evidence, and
it is recorded here so that it is not quietly forgotten.

This corrects the entry above it: de-duplication makes a push invisible **when
catch-up has already delivered the operation**, which is what the laptop saw
first. It does not make pushing useless — the follower on phone B saw the
pushes arrive.

### ✅ Stop deriving the live count; report p2panda's own

A fourth wrong version, and then the decision to stop having a version at all.

The boolean rule still over-reported: a phone said `live=8894` of
`received=8915` while its only publisher had pushed four times. The laptop said
`live=73` while every logged `SyncFinished` from all three peers showed
`received_live_operations: 0`.

What settled it was making the code name the peer behind every live arrival —
one line per arrival, capped. Two facts fell out at once:

* **every live arrival came from `b8e0c9ba`**, the one peer running the fix;
  none from `9eeeac47` (the pre-fix loop phone) or `d4dd64f2` (a follower that
  publishes nothing). The signal is real and it is attributable;
* the raw counter advanced **`n=2,4,6,8,10,12,14,16…`** — two per event this
  stream observes. No per-event rule can be right against that, in either
  direction.

So there is no longer a rule. `live_seen` holds each open session's counter
exactly as p2panda last reported it, `live_retired` accumulates sessions that
have ended, and `live_received()` adds them. It counts per session, so an
operation delivered live on two sessions counts twice — which is what "live
operations received" means, and is preferable to a fifth attempt at guessing.

The `live op from <peer> n=<count>` lines make the number checkable against a
publisher's own `pushed=`, which is the only external check it has ever had.

**Five versions of one diagnostic.** The counter was never the product; it was
supposed to be the instrument for confirming the product. It cost more than the
fix did, and the reason is worth keeping: a derived number whose failure mode is
over-reporting will be believed, because it agrees with what you hoped.

⚠️ The `live <= received` assertions are gone, deliberately. The two now measure
different things — p2panda's live arrivals per session against operations this
peer stored — so that assertion would be wrong rather than protective. The check
that does hold is the follower's live count against the **publisher's** own
count of what it sent, and it lives in `tests/two_process.rs` where both sides
are visible at once.

### ⚠️ A fifth way to be wrong about the live counter: the units

Not a new derivation — version 5 is right to report p2panda's number and derive
nothing. The problem is what it is printed next to.

`live_received()` sums `Metrics::received_live_operations` across sessions, and
this file already records what that counter does: it advances `n=2,4,6,8,…`,
**two per event the stream observes**. `received()` is one per operation stored.
So `live` is roughly twice the live *events* and `received` is once per
operation — two different units, printed on one line under one word:

```
sync: counts received=15632 live=15531
```

Phone B, 2026-09-14 16:32, on released Ayni 0.1.5. Read naively that says 99% of
arrivals were pushed, which it does not say and cannot. It is not an
over-report in the sense the four earlier versions were — nothing is being
reconstructed — but it invites exactly the comparison those four were wrong
about, and the arithmetic guard `live <= received` that was so hard-won is
meaningless between quantities in different units.

**It also disagrees with the negative control recorded earlier the same day**
(`received=4 live=0`, "the right answer"), and nobody has established which of
the two readings is the surprising one.

Not fixed here, deliberately. Four attempts to be clever about this counter were
wrong; a fifth made in passing, while changing something else, would be the same
mistake. What it needs is a decision about what is being asked — "how many
operations arrived by push" is answerable only if the doubling is divided out,
and whether it is exactly two every time is an upstream fact nobody has measured
rather than inferred. Until then the two numbers should at least not share a
label.

### ✅ CLOSED — the loop phone's publisher reaches Ayni, 2026-09-14 16:47

The open item both handovers led with. The loop phone was updated at 16:44 and
Ayni rebuilt and installed at 16:45, and the two ends agree without either being
asked to agree with the other:

| time | peer | line |
|---|---|---|
| 16:44:23 | loop phone, **pre-fix** build | `shadow agrees — … failures 0` — no `pushed` field exists |
| 16:45:27 | loop phone, fixed | `shadow agrees — … failures 0, **pushed 1**` |
| 16:46:41 | loop phone, fixed | `shadow agrees — … failures 0, **pushed 2**` |
| 16:47:47 | Ayni, phone B | `sync: **live op from 9eeeac47** n=1` |

`9eeeac47` is the loop phone. **That is a push from the phone driving the pump,
delivered to the follower app** — the path that has never worked in any build
this project shipped, and the one every other measurement was a proxy for.

The follower process started at 16:45:46 and recorded the arrival at 16:47:47,
which is discovery time on a fresh process rather than push latency. Nothing here
measures freshness yet.

**The evidence is two independent sources.** The publisher's `pushed=` is its own
count of `SyncHandle::publish` returning; the follower's is p2panda's per-arrival
attribution, naming the peer. Neither is derived from the other, which is the
property the four wrong counters lacked.

### 🔍 And `n=1`, which refines the units problem rather than settling it

The entry above this one says the raw counter advances two per event. This
arrival reported `n=1`. So the doubling is **not constant** — the plausible
reading is that it counts once per topic-session observing the operation, and a
follower carrying two logs of a subject sees it twice, but that is a hypothesis
and nobody has measured it upstream.

Which makes the practical rule sharper, not looser: **the magnitude of `live` is
not interpretable and should not be compared to `received`.** What is
interpretable is `live op from <peer>` — one line per arrival, naming who sent
it. That is what closed this item, and it is what a freshness measurement should
be built on.

### ✅ D28 on hardware, the same pass

```
keys holders for 552f688a: 1 (9eeeac47)
```

A follower learning from bucket gossip that another phone holds a subject's
**keys** logs. That line was `none` by construction until today — nothing
announced keys subjects and nothing adopted them. Two phones, so this is the
mechanism working, not the redundancy being proven: that still needs four or
more devices and disjoint shares.

### 📊 Freshness, measured at last — median 24s, and one gap I cannot explain

The thing every counter in this project was a proxy for. **It needed no new
code**: `SyncWorker` already logs `keys newest for <subject>: Ns old`, which is
the newest readable reading's own timestamp against wall clock. Not derived from
`received`, not derived from `live`, and not derived from anything written
today — which is why it is the number to use.

Sampled once per two-minute pass on phone B, 2026-09-14:

| window | n | min | median | max |
|---|---|---|---|---|
| both ends pre-fix, 16:39–16:44 | 7 | 89s | **268s** | 400s |
| both ends fixed, whole window | 13 | 23s | 83s | 345s |
| both ends fixed, settled 16:55–17:17 | 13 | 23s | **24s** | 107s |

**Read the third row, not the first two.** The pre/post comparison is confounded:
the loop phone was updated at 16:44 and Ayni at 16:45, so the "pre-fix" samples
straddle the change and the early post-fix ones include a cold follower
discovering its peers. The settled row is twelve minutes of a warm follower with
a fixed publisher, and it says the newest reading is **23–83 seconds old**.

Twenty-two minutes, thirteen samples, one per pass:

```
23 23 23 24 24 24 24 80 83 83 83 84 107
```

For scale: the follower polls every 120s and the sensor reports every ~60s, so a
median of 24s means the data is arriving between polls rather than at them —
which is what pushing was supposed to buy and what, before `634acaf`, it could
not have been buying, because nothing ever called `publish`. **All thirteen
samples are under the 120s poll interval**, including the worst.

✅ **BOTH OPEN QUESTIONS CLOSED, 17:22, once the loop phone was back on adb and
its `sealed epoch` times could be put beside the follower's series.**

The publisher seals on a **60-second cadence**, with extra partial seals between
— and it had a **171-second gap** at 17:19:35 → 17:22:26.

| follower sample | reported age | publisher's last seal | follower actually behind by |
|---|---|---|---|
| 17:13:47 | 83s | 17:13:23 | **59s** |
| 17:15:47 | 84s | 17:15:23 | **60s** |
| 17:17:47 | 24s | 17:17:23 | **0s** |
| 17:19:47 | 23s | 17:19:35 | **11s** |
| 17:21:47 | **143s** | 17:19:35 | **11s** |

**1. The bimodality is exactly one seal cycle.** 84 − 24 = 60s, the publisher's
own interval. The follower is either current with the last seal or one seal
behind it, depending on whether a push landed before the sample ran. There was
never a second mechanism to find.

**2. The long samples are not staleness.** At 17:21:47 the follower reported
143s and was **11 seconds behind everything the publisher had sealed**. The age
climbed because the subject stopped sealing for 171s, and a follower cannot be
fresher than the subject seals. The 345s excursion at 16:51–16:54 is almost
certainly the same thing; its publisher logs had rolled out of the buffer.

**So the honest statement of transport freshness is not the raw age series.** It
is: **the follower is never more than one seal cycle behind the publisher, and
usually within about eleven seconds.** The raw age is that lag plus whatever the
publisher's own cadence adds, and on this hardware the publisher's cadence is
the larger and more variable term.

✅ **And that remaining question was measured too, ten minutes later — it is
the sensor, and there is nothing left to optimise.**

The entry above ended by relocating the work onto "why AAPS's drain seals at 60s
and sometimes stalls for three minutes". Both halves of that sentence turned out
to be wrong, in the same direction: the drain does neither.

**The drain adds 74–228 milliseconds.** It seals on the reading:

| CGM inserted | sealed | latency |
|---|---|---|
| 17:13:23.346 | 17:13:23.461 | 115 ms |
| 17:15:23.048 | 17:15:23.276 | 228 ms |
| 17:19:23.968 | 17:19:24.073 | 105 ms |
| 17:22:26.870 | 17:22:26.993 | 123 ms |

**The 60-second cadence is the Libre 3.** Inter-arrival on the loop phone:
60.1, 59.6, 60.0, 60.4, 59.6, 60.8 seconds. The publisher is not on a timer; it
seals what arrives, when it arrives.

**And the 171-second seal gap was a 183-second SENSOR gap** — 17:19:23 → 17:22:26
with no reading inserted at all, followed by a backfill burst of five readings at
17:22:28. The publisher had nothing to seal. Nothing stalled.

**So the whole chain is now accounted for**, and the budget is:

```
sensor 60s cadence  +  drain ~0.1s  +  transport ≤1 seal cycle, usually ~11s
```

The transport was the entire subject of two sessions and is now the *smallest and
least variable* term. What a follower shows is dominated by the sensor's own
delivery — including its multi-minute gaps, which no work in this repository can
shorten. A median follower age of ~24s against a 60s sensor is close to the floor.

**The useful consequence is for wording, not for code.** A follower showing "3
minutes ago" during a sensor gap is displaying the truth, and the age on the hero
card is the right design precisely because of this: the number is old because the
*reading* is old, not because the network is broken, and only showing the age
lets a person tell those apart.

⚠️ **One excursion, unattributed: 202s → 322s → 345s across 16:51–16:54.** It
climbs at exactly the rate of elapsed time, which means no reading arrived at
all for about five minutes. Candidates: the publisher settling after its update,
a missed push, or a sealing gap on the publisher's side — the follower cannot
be fresher than the publisher seals. **Distinguishing them needs the publisher's
`sealed epoch` timestamps beside this series**, and the loop phone dropped off
adb before they could be collected. Left unattributed rather than guessed at.

> **And the disconnection turned out to demonstrate the product.** With no adb
> to the loop phone at all, the follower kept reporting
> `live op from 9eeeac47 n=9,10,…,14` and `keys newest for 552f688a: 26s old` —
> so the publisher was known to be alive, looping and pushing, from the other
> phone, through the swarm. The USB drop was host-side and nothing else.
>
> Worth noticing because it is the actual use case: the question a follower
> exists to answer is "is their phone still working and how current is this
> number", and it answered it about a phone the laptop could no longer see.

**What this still does not measure:** anything overnight, anything in doze,
anything off-LAN, and the tail. Twelve minutes of a plugged-in phone on wifi is
the easiest case there is.

### 🐛 A test was asserting the mistake it was written to catch

Found while extracting `share::carry_share` for the desktop peer
([D29](decisions.md)). `two_process.rs` had:

```rust
assert!(got <= sent, "the follower counted {got} pushed arrivals from a
                      publisher that pushed {sent}")
```

added deliberately to catch a transport diagnostic that over-reports — the
defect four versions of the live counter had. **It is not an invariant.** `sent`
is what `broadcast` returned, which is the number of *topics* published on;
`got` is p2panda's `received_live_operations`, measured at one increment per
event on one build and two on another. Neither counts operations, so neither
bounds the other. It failed about **one run in two**, only when the whole file
ran, which reads exactly like load flakiness — and it was already there before
any of today's work.

**Two further attempts failed for the same underlying reason**, which is the
part worth keeping:

| attempt | why it was wrong |
|---|---|
| `live <= sent` | different units, as above |
| `received <= ops`, ops read at first push | compared a late follower reading against an early publisher one |
| `received <= ops`, both late | `received` counts *store events* and counts one operation twice if it arrives on two sessions |

> **Every counter in this replicator counts events, not things.** `received`
> per store, `live` per p2panda increment, `sent` per topic. That is fine for
> "is anything happening" and useless for "how many", and three assertions in a
> row were built on the assumption that one of them could bound another.

**The fix asks the store.** `keyspeer` now reports `segments` via
`get_log_size`, and the ceiling is that a follower cannot *hold* more operations
of a subject than its publisher created — distinct things on both sides. **3/3
stable**, against 1-in-2 before.

⚠️ **And it would have started failing on `main`.** The crates CI added earlier
the same day runs `diaswarm-net`; a 50% flake there is worse than no test,
because it teaches everyone to re-run rather than read. It passed its first two
CI runs by luck.

### 🔴 THE OVERNIGHT FAILURE, FOUND: Android time-limits the foreground service

**2026-09-15 03:37:54.** The soak answered, and the answer is not doze.

```
E ActivityManager: FGS Crashed: ServiceRecord{… nz.diaswarm.follower.StayAwake}
E AndroidRuntime: android.app.RemoteServiceException$ForegroundServiceDidNotStopInTimeException:
    A foreground service of type dataSync did not stop within its timeout
I ActivityManager: Process nz.diaswarm.ayni (pid 14237) has died: prcp FGS
```

and then, one second later, it could not come back:

```
E AndroidRuntime: android.app.ForegroundServiceStartNotAllowedException:
    Time limit already exhausted for foreground service type dataSync
W ActivityManager: Scheduling restart … in 1800000ms for start-requested
```

**Android 15+ time-limits a `dataSync` foreground service to about six hours in
any twenty-four.** At the limit the system calls `Service.onTimeout()`; a
service that does not stop itself has its **process killed**, and the type's
quota is then spent — so every restart for the rest of the window is refused.
Ayni declares `android:foregroundServiceType="dataSync"`, targets SDK 36, and
implements no `onTimeout()`.

The service started at 21:25 and died at 03:37:54: **6 h 12 m.**

#### What it means, and it is the flagship use case

**Ayni cannot watch overnight.** A parent following a child's glucose has the
app die around the six-hour mark and stay dead. That is the product's central
promise, and it fails on a stock phone with nothing misconfigured.

It fails *safely* — the app is gone rather than showing a stale number as
though it were current, which is what showing the age was for — but it fails.

#### It corrects a finding this repository already recorded

`docs/migration.md` and the project memory both say the foreground service is
**sufficient**, on the strength of *"51 minutes in natural deep IDLE, 26 of 26
fetches, `effective=NONE` throughout"*. Every word of that is true and the
conclusion does not survive six hours. **A 51-minute measurement cannot see a
six-hour limit**, and the earlier soak that might have — 17:46→19:46 — was two
hours and also too short.

#### And doze was never the problem

Worth stating plainly, because two sessions were spent on it. Between entering
deep IDLE at 21:40 and dying at 03:37, the follower ran its **two-minute cadence
without interruption for 5 h 57 m**, standby bucket `10 (ACTIVE)`,
`effective=NONE` throughout. One 42-minute gap at 23:59, otherwise perfect.
**Doze did not suppress it. The platform killed it.**

`effective=DOZE|APP_BACKGROUND` at 03:43 is a *consequence* of there being no
process left holding a foreground service — not a cause, and not the failure the
`relay-drops-overnight` note describes.

#### Fix directions, none verified

* **Implement `Service.onTimeout()`** so the service stops cleanly instead of
  being killed. Necessary regardless — a crash loop is worse than a stop — but
  it does not buy availability, because the service still stops.
* **Change the FGS type.** The six-hour limit applies to `dataSync` (and
  `mediaProcessing`); other types are not time-limited. `specialUse` is the
  honest declaration for "hold a p2p connection open", and its Play Store review
  requirement does not apply to an F-Droid app. **Verify against the current
  platform docs before relying on this.**
* **Drop the FGS and use the battery-optimisation exemption instead**, which is
  the only thing that clears `DOZE` and is the user's to grant. Costs the
  `APP_BACKGROUND` clearance an FGS provides, and puts the cadence on
  WorkManager's 15-minute floor.

**Do not pick one from this list without measuring it for more than six hours.**
That is the whole lesson of this entry.

### 📏 How long the blackout lasted: 220 minutes, and only because somebody intervened

The watch left running after the kill closed at 07:18 with `RECOVERED … after
220m down`. **That number is a lower bound and an artefact.**

Ayni came back because the app was rebuilt, reinstalled and launched at 07:13.
Android's documentation is explicit that *"the timer resets when the user brings
the app to the foreground"* — and nothing else was going to do that at four in
the morning. Every automatic restart in between was refused. Left alone, the
blackout would have run until somebody opened the app, or until the six hours
aged out of the rolling twenty-four-hour window: **on the order of half a day,
not three hours.**

Which is the shape of the failure that matters. Not "the follower is briefly
stale" but "the follower is gone, it will not come back, and the only thing that
revives it is the person who was asleep."

### ✅ The fix, and what it has and has not proven

`specialUse` is declared, verified in the binary (`0x40000000`, was
`0x00000001`) and live on the device
(`isForeground=true types=0x40000000`). `onTimeout` is implemented as a clean
stop in case a limit is ever applied to this type too.

⚠️ **Nothing is proven yet.** A soak started 07:13 and has to pass **372
minutes** — where `dataSync` died — before this is more than a plausible change.
`scratchpad/fgs-soak.log` records a line every ten minutes; the monitor reports
the process dying, a restart, any `onTimeout`, and the 6h and 12h marks.

**And the thing that nearly invalidated it before it began:** the first build of
this fix was declared done on the strength of a grep that found no `error:`
lines. Gradle had in fact failed — `ANDROID_NDK_HOME is not set`, because
`./gradlew` was invoked directly instead of through `follower/build-apk.sh` —
the APK was still the previous night's, and its manifest still said `dataSync`.
Checking the *binary* rather than the source is what caught it. A six-hour soak
would otherwise have run against an unchanged app and reported success.

### 🐛 A desktop peer catches up once and then never again

Found 2026-09-15 by running `diaswarm-peer` on a laptop for longer than the five
minutes it had ever run before, and asking the obvious question: the loop phone
seals every minute, so why is the count static?

**What a healthy-looking peer was actually doing:**

```
pool 4 · carrying 5 (+0) · holding 1739 · pushes not yet · 6.2 MB · STALLED 3 passes
    sync: SyncFinished { … received_sync_operations: 10, received_live_operations: 0 } from d4dd64f2
    sync: LiveModeStarted from d4dd64f2
    sync: SyncFinished { … received_sync_operations: 10, received_live_operations: 0 } from d4dd64f2
    sync: LiveModeStarted from d4dd64f2      ← identical, every pass
```

1. **It syncs with exactly one peer, `d4dd64f2` — which is Ayni**, the follower
   that publishes almost nothing. It never opens a session with `9eeeac47`, the
   loop phone, which is the only peer producing data.
2. **The same ten operations arrive every session.** `inbound_sync_bytes: 7215`
   identical each time — a re-sync loop that never advances.
3. **`received_live_operations: 0` on every session**, while `LiveModeStarted`
   fires each time. Live mode starts and delivers nothing.

Its 1,716 operations of the loop phone's log are all from *before* a restart, so
a session with the publisher does happen at some point and then never recurs.

**This is the receiving-side mirror of the defect that cost two sessions.** The
original was a publisher that never called `SyncHandle::publish`; this is a
subscriber that starts live mode and receives nothing from it. Both present as
healthy totals with no fresh data, which is the shape this project keeps
producing and keeps failing to notice.

**Why nobody saw it:** the desktop peer had run for five minutes, twice, in
testing. A peer that catches up on connect looks perfect for five minutes.

#### The diagnostic that was missing, now present

`diaswarm-peer` printed `Replicator::received()` and nothing else. Three changes:

* **`holding` comes from the store**, via `get_log_size` over carried subjects —
  not from `received()`, which counts *store events* and therefore double-counts
  while the D31 transition carries each subject at two topics. It read **3,400
  against 1,729 actually held**, and a number twice the truth looks like health.
* **`--events N`** prints the sync event log, and a stall prints it unprompted —
  the diagnostic that took a day to build for the phones and was never wired
  into the peer.
* **A stall detector**, because "nothing new is arriving" cannot be seen in a
  cumulative count and is the only thing that matters.

⚠️ **Also observed and not investigated: 857 MB peak RSS, 537 MB swap** for a
peer holding 6 MB, over twenty-one minutes (`systemd` accounting). That is its
own problem and it is not this one.

*Open. Root cause not established.* The next question is why a session with the
publisher is not re-established, and why `LiveModeStarted` yields no live
operations on this peer while the same code delivers them to Ayni.

### 📏 Peer memory: fine at rest, spikes with catch-up

Measured on the laptop peer, 2026-09-15.

| | |
|---|---|
| steady state | **119 MB** RSS, 0 swap, 65 threads, 11 MB on disk |
| during initial catch-up of 3,400 operations | **857 MB peak, 537 MB swap** |

**An earlier note here called that 857 MB "for a peer holding 6 MB", which
conflated a transient with a resting cost.** At rest the peer is unremarkable.
The spike belongs to bulk catch-up.

**The ratio is the part worth keeping.** Roughly 250 KB of peak memory per
operation caught up, against about 7 KB per operation on disk — **35×**. That
shape suggests the sync path buffers rather than streams.

It does not bite at this size and it is squarely in the way of the thing this
design is for. A peer adopting a subject with a year of Libre 3 history is
catching up on the order of **half a million operations**, two orders of
magnitude beyond what produced the 857 MB. Anyone sizing a volunteer carrier —
a Pi, a NAS, an old laptop — needs this number before they are told 329 MB of
disk is the cost.

Not investigated. Recorded because the measurement exists now and will not
after the process restarts.

### 🔍 Why the peer stalled: what the upstream source says, and what it does not

Chased through `p2panda-net` and `p2panda-sync` 0.7.1 after the desktop peer
caught up once and then never again.

**1. A sync session is created only from HyParView membership events.**
`sync/actors/manager.rs::spawn_membership_task` derives a *separate gossip
overlay per sync topic* and initiates a session on exactly two events:

```rust
GossipEvent::Joined      { topic, nodes } => InitiateSync
GossipEvent::NeighbourUp { node,  topic } => InitiateSync
```

So **the peers you exchange data with are exactly your active view**, per topic.
There is no other path: `SyncHandle::initiate_session(node_id)` exists, does
precisely "sync with this peer", and is `#[cfg(test)]` behind a TODO asking
whether to make it public.

**2. Relay between sessions is same-topic only.** The protocol doc says live
messages are "forwarded to any concurrently running sync sessions", which reads
as unconditional. `manager/event_stream.rs` shows it is not:

```rust
let topic = state.session_topic_map.topic(session_id);
let keys  = state.session_topic_map.sessions(topic);   // same topic only
```

Two peers can be connected and never relay, if their sessions are on different
topics.

**3. Together, these mean topic proliferation dilutes connectivity.** Each topic
is its own overlay with its own sampling, and relay cannot bridge across topics.
Finer-grained topics — which feel tidier — make both mechanisms sparser. The
[D31](decisions.md) transition, which carries every subject at *two* topics
during the migration, doubles the overlay count and therefore halves the density
of each.

#### The knobs that were never configured

`Gossip::builder(..).config(GossipConfig)` was never called, so membership ran
on iroh-gossip's defaults: `active_view_capacity: 5`, `shuffle_interval: 60s`
— the latter commented **"Wild guess"** upstream. `DIASWARM_ACTIVE_VIEW` and
`DIASWARM_SHUFFLE_SECS` now expose both.

⚠️ **AND THE EXPERIMENT THAT WOULD HAVE TESTED THEM WAS CONFOUNDED, WHICH IS THE
POINT OF THIS SECTION.** Raising the active view to 16 appeared to produce a
peer climbing `+1` per pass with no stall. The log timestamps say otherwise:

```
09:09:44  pid 1614426  (OLD binary, default config)  holding 1795
09:11:44  pid 1614426                                holding 1797   ← already +1/pass
09:12:33  pid 1698373  (NEW, active_view=16)         starting
```

**The baseline was already healthy.** The stall is intermittent, and measuring a
fix against a period when the fault was absent proves nothing. A real test needs
the stall reproducible on demand, or hours of A/B — not two lines that came from
the binary without the change.

**Nothing in points 1–3 depends on that experiment.** They are read from
upstream source and stand on their own; the view sizing remains a plausible
contributor and an unproven one.

### ✅ CLOSED — the keys vault delivers off-LAN, 2026-09-15 10:02

Listed as open in the handover: *"Nothing is measured off-LAN on the fixed
build. The relay path was proven on 2026-09-14 for the **core** vault; the keys
vault's live push has only ever been seen on one wifi."*

The loop phone's wifi was switched off at 10:01:57. The laptop peer, on home
wifi, carrying that subject explicitly:

```
10:01:57  holding 1833   ← wifi off
10:02:57  holding 1834
10:04:57  holding 1835
10:05:57  holding 1836     pushes yes throughout, no STALLED
```

Unchanged rate, roughly one operation a minute, matching the publisher's seal
cadence. The operations that grew are `b6b573c6…`, the loop phone's own keys
subject — not another peer's.

**And it is a cleaner test than it was meant to be.** The loop phone dropped off
adb over USB during the window, so there was no local channel left that could be
mistaken for the relay: phone on mobile data, laptop on home wifi, no shared
network, no mDNS, data still arriving.

The phone's wifi state is the user's report — adb was gone, so it could not be
read off the device. They confirmed it directly: *"it's definitely off."*

### ✅ CLOSED — `specialUse` survives the night, 2026-09-15 13:29

The soak passed the mark that killed the old build.

| | old (`dataSync`) | new (`specialUse`) |
|---|---|---|
| lifetime | **killed at 372m** | **376m and counting, same pid** |
| after the kill | restarts refused, dead 220m until a human opened it | n/a |
| `effective=` | `DOZE\|APP_BACKGROUND` once the process died | `NONE` throughout |
| timeout exceptions | `ForegroundServiceDidNotStopInTimeException` | **zero** |

`pid 19707` from 07:13 to 13:29 — never killed, never restarted, `doze=IDLE`
throughout, cadence still running. Verified on the device rather than inferred:
process id unchanged, `types=0x40000000`, and a `logcat -T` bounded to after the
fix went on showing **no** timeout of any kind.

**So the flagship use case works.** A follower can watch somebody's glucose
overnight, which it could not do yesterday and had never been tested for long
enough to discover.

And the rule that found it stands: *never conclude anything about overnight
behaviour from a run shorter than the claim.* A 51-minute measurement and a
2-hour one had both been read as proving the foreground service sufficient.

### 📏 Catch-up memory, measured properly — and my earlier ratio was wrong

An earlier entry here put peak memory at "roughly 250 KB per operation caught
up… 35x" and projected half a million operations as ruinous. **Measured against
a control, that is not the shape of it.**

| operations caught up | retained RSS |
|---|---|
| **0** — carries only its own subject, nothing to fetch | **42 MB** |
| 1,042 | **881 MB** |
| 1,962 | **835 MB** |
| the long-running service, before its catch-up | 119 MB |
| the same service, after | **400 MB** |

**It is not proportional to operation count.** Half the operations cost slightly
*more* memory. So the per-operation ratio was an artefact of dividing by a
number that was not the driver, and the "200 GB for a year of Libre 3"
projection that followed from it is withdrawn.

**What is solid, and is still a problem:**

* a peer that performs any substantial catch-up allocates **hundreds of
  megabytes** — 400 to 880 MB across these runs;
* **it is never released.** RSS sits flat at the high-water mark for as long as
  the process lives. This is not a transient spike;
* a peer that fetches *nothing* stays at **42 MB**, so the cost belongs entirely
  to catch-up and not to being a peer.

**Why it matters, in the form that is actually true:** the natural hosts for a
volunteer carrier are a Pi, a NAS, an old laptop — 1 to 4 GB of RAM. One
adoption of a single subject can take most of a gigabyte and keep it. Several
subjects, or an unlucky restart, and the thing is a memory hog on a machine
somebody donated. That is a deployment problem now, not a scaling problem later.

*Not investigated further.* The variance — 400 MB in one run, 880 MB in another,
for comparable work — means the driver has not been identified and per-operation
arithmetic will keep producing wrong answers until it is.

### 🧪 A/B running: does a larger active view reduce stalls?

**Started 2026-09-15 13:52. Result not in.**

Sessions are created only when a peer enters this node's HyParView active view
(see the upstream draft). Default capacity is **5**. With a pool of four, raising
it should mean every peer is always in view, so sampling stops deciding who we
sync with — *if* that is what causes the stalls.

**Baseline, default config, 09:54 → 13:52 (~4 h):**

| | |
|---|---|
| passes | 238 |
| stalled passes | **18 (7.6%)** |
| re-subscribes triggered | 6 |

**Test config:** `DIASWARM_ACTIVE_VIEW=24`, and **nothing else**.

⚠️ **One variable, deliberately.** The previous attempt set
`shuffle_interval=15s` at the same time, which broke announcement delivery
outright — the peer sat at `heard 0 · carrying 1 · holding 0` for twenty minutes
and had to be reverted. It also ran against a period when the fault was absent,
so it could not have shown anything either way. Both mistakes are why this one
has a measured baseline and a single knob.

**What would settle it:** a comparable window with materially fewer stalled
passes. If the rate is unchanged, sampling capacity is not the cause and the
upstream ask is the only real fix — which is worth knowing before anyone spends
time on configuration.

**What it cannot show:** anything about pools larger than the active view, where
this mitigation is unavailable by definition.

### 🔴 Ayni leaks, and the low-memory killer takes it at ~8 hours

**2026-09-15 15:21:51**, phone B, 482 minutes into the `specialUse` soak:

```
lowmemorykiller: Kill 'nz.diaswarm.ayni' (19707), uid 10253, oom_score_adj 200
    to free 5249980kB rss, 15764kB swap;
    reason: min watermark is breached and swap is low
ActivityManager: Process nz.diaswarm.ayni (pid 19707) has died: prcp FGS
```

**5.2 GB resident**, on a device with 7.6 GB. A fresh process is **120 MB**. So
it grew by a factor of forty over eight hours.

#### This does not undo the `specialUse` fix — it is a different bug

Worth being exact, because the claim made this morning was "the fix is proven"
and it still is, for what it claimed:

| | `dataSync` (yesterday) | `specialUse` (today) |
|---|---|---|
| cause of death | foreground-service **time limit** | **low-memory killer** |
| timeout exceptions in log | `ForegroundServiceDidNotStopInTime` | **none, at 8 hours** |
| after death | restarts **refused** — dead 220m until a human opened it | **restarted automatically**, `effective=NONE`, back in a second |
| recovers alone | ❌ | ✅ |

The foreground-service type was the right diagnosis and the right fix: no
timeout fired in eight hours where the old build died at six. **The overnight
test simply surfaced a second, independent bug underneath the first** — which is
what happens when a thing is run for longer than it has ever been run before.

#### But the flagship use case still does not work

A follower that is killed every eight hours is better than one that dies at six
and stays dead, and it is still not a thing somebody can rely on overnight. The
severity is lower — it self-heals, and the app shows the *age* of a reading so a
stale value never masquerades as current — but the honest statement is: **Ayni
cannot yet watch through a night without being killed.**

#### And it connects to the carrier measurement

The laptop peer measurements recorded above — 42 MB with nothing to fetch,
400–880 MB after a catch-up, never released — looked like retention that
plateaued. Over eight hours on a phone it plainly does not plateau. **They are
the same defect seen over different durations**, and the laptop numbers were
taken over minutes rather than hours, which is why they read as a plateau.

*Root cause not established.* What is known: it is not the foreground-service
type, it is not catch-up size (measured: not proportional), and it grows for as
long as the process lives.

### 📉 A 70-minute publisher outage, off-LAN — and a clean recovery

**2026-09-15, loop phone unattended on mobile data, wifi off, nobody touching
it.** Sharing stopped at ~15:10 and resumed at 16:20 without intervention.

Both readers saw it independently, which is what identifies the publisher rather
than either reader as the cause:

| | during | on recovery |
|---|---|---|
| laptop peer | `holding` frozen at 2025, 56 stalled passes, `pool` 3 → 1 | **+44 in one pass**, then steady +1/min |
| Ayni (phone B) | `3471s old` and climbing by elapsed time | `113s` → `52s` within two passes |

**Nothing was lost.** The backlog arrived complete on reconnect, which is the
catch-up path doing exactly its job.

**And Ayni showed `3471s old` the whole way through** rather than a stale number
presented as current. That is the design decision about always displaying a
reading's age earning itself — the dangerous failure for a follower was never an
error on screen, it was a plausible value that is an hour out of date.

⚠️ **Cause not established.** The phone was on mobile data, unattended, and
recovered by itself — a pattern consistent with doze network-blocking, released
at a maintenance window. It cannot be confirmed: the phone was off adb for the
whole window, and `dumpsys netpolicy` is the only thing that names the flag.

**If it is doze, it is the most important open question here**, because it is
the actual overnight case: a looping phone in somebody's pocket or beside a bed,
on mobile data, sharing with a parent who is asleep. An hour-long silent gap in
that scenario is the thing the follower's age display exists to make survivable —
but survivable is not the same as working.

---

## The cause, established — and it was not doze

**2026-09-15, same day, from `AndroidAPS.log`.** The durable log covers the
window; logcat had rolled past it hours before, which is the whole reason AAPS
keeping its own log on disk mattered here.

The outage has a single, exact signature:

```
15:04:24  pool 3 peers, 4 buckets, holding 3, relay=connected(1)
15:05:25  pool 2 peers, 2 buckets, holding 3, relay=disconnected
   ...    88 consecutive passes
16:16:23  pool 2 peers, 2 buckets, holding 3, relay=connected(1)
```

Seventy-one minutes, bracketing both readers' observations exactly.

**The publisher never faltered.** Into that hole AAPS sealed **84 epochs**,
`holds` climbed 1470 → 1560, and every single pass reported `missing 0, lost 0,
failures 0`. The data was made and it was kept. It had nowhere to go — which is
precisely why nothing was lost and the backlog arrived complete.

⚠️ **Doze is ruled out, not merely doubted.** AAPS is on the battery
optimisation whitelist (`user,info.nightscout.androidaps,10310`), it runs at
`targetSdk 32` and is exempt from the foreground-service regime entirely, and it
was demonstrably working throughout — a phone sealing an epoch a minute is not a
phone Android has stopped. The previous entry's suspicion was reasonable and
wrong. That makes **three** overnight failures now blamed on doze and caused by
something else.

`relay=disconnected` is not our inference either: it is iroh's own
`home_relay_status()`.

### Why it happened

`netwatch`'s Android route monitor is a deliberate stub —

```rust
// Very sad monitor. Android doesn't allow us to do this
```

— and its wall-time poll is stretched to an hour on mobile for battery, and
fires on a *clock* jump rather than a network one. **So on Android iroh cannot
see a network change at all.** Its own docs name the platform and the remedy:
`Endpoint::network_change()` is public and exists for exactly this. We had never
called it.

What that costs is everything in `handle_network_change`: the UDP rebind,
`dns_resolver.reset()`, a fresh net report, and the QUIC stack being told to
migrate. The relay is a **hostname**, so reconnecting needs DNS — and a resolver
still aimed at the network the phone walked away from keeps failing until it
walks back. Which is what 16:16 is.

### Why every previous test passed

Switching wifi off at home and on again keeps working, and that observation is
what makes this specific rather than hand-waving. Live QUIC connections keep
flowing over the wildcard socket via cellular, and nothing has to re-resolve
anything. **The failure needs a network change that outlives its connections** —
leaving, not toggling. Every off-LAN test so far was run from the house.

### What was recorded too strongly

"Off-LAN is proven" has been in the record since 2026-09-11. It was measured
with the phone on mobile data **while still at home**. Off-LAN via relay is real
and the relay did come back — but it depended on a notification that was never
being sent, so it was never as robust as the record claimed. See D32.
