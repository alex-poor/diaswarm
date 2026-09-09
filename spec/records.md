# The canonical record stream

**Status: v3 — 2026-09-09.** Every later stage encodes against this.

> **v3 moved the epoch boundary off UTC.** v1 and v2 cut days at UTC midnight,
> on the argument that a constant offset keeps epoch identity unambiguous. That
> argument is right and is kept — but it defended the *width* and said nothing
> about the *phase*, and the phase was wrong. See §5.1.

> **v1 was wrong about durations and lasted one day.** It said `dur` is minutes.
> AAPS stores every duration in **milliseconds** — a 15-minute temp basal is
> `900000` — and the emitter passed the column through verbatim, so a v1 stream
> declared minutes and carried milliseconds. A consumer applying the
> specification would have read a 15-minute temp basal as **625 days**.
>
> **The bytes do not change; the contract does.** The fix is that the document
> now says what the data always was. The version is bumped anyway, because a
> consumer that implemented v1 correctly was wrong, and §5.2 exists precisely so
> that is detectable rather than discovered by getting wrong answers. Nothing had
> been published, so no compatibility window is owed — which is the only reason
> this was cheap.
>
> It is the same defect as the mmol/mg-dL one in §2, found the same way: by
> reading what the device actually writes rather than what a table looked like it
> meant.
Changing it now costs a compatibility window, which is the point of freezing it:
the sealing layer, the AAPS plugin and the commons gateway can all be built
against a contract that will not move under them.

A stream declares the version it conforms to, in its own first line (§5.2), so a
v2 is a thing a consumer can detect rather than a thing it discovers by getting
wrong answers.

Produced by [`tools/canon.py`](../tools/canon.py). Consumed by the sealing layer,
the AAPS plugin and the commons gateway.

---

## 1. What a record is

One event, at one instant, as the device actually recorded it. Not a database
row — see §3.

```json
{"k":"cgm","mgdl":163.0,"src":"Dexcom G6","t":1782938503230,"trend":"FLAT"}
{"basal":false,"k":"bolus","t":1782938700000,"type":"NORMAL","u":1.25}
{"g":30.0,"k":"carb","t":1782938700000}
```

**Keys are sorted and field order is not semantic.** The canonical encoding
sorts them so that two implementations emit byte-identical lines for the same
record, which is what makes a stream diffable and hashable. Read records by
name, never by position.

- **`t`** — epoch milliseconds, UTC. The sort key and the only mandatory field
  besides `k`. Local time is a rendering concern; a stream that carries local
  time cannot be merged across a timezone change, and people travel.
- **`k`** — the kind. A closed, versioned vocabulary (§2).
- Everything else is per-kind and **optional**: a missing field means the device
  did not report one, which is different from zero and must stay different. The
  third line above is a carb entry the device gave **no** duration for — so it
  carries no `dur` at all. A carb entry with a real duration of zero carries
  `"dur":0`. An emitter that writes an explicit `null`, or that helpfully
  substitutes a zero, has destroyed that distinction for every consumer
  downstream.

**Ordering** is by `(t, k)`, and ties break on the record's own canonical
encoding. Ties are real — a bolus and its carbs share a timestamp, and two carb
entries in one millisecond are possible — so `k` alone is not enough. Without the
third key, equal-`(t, k)` records fall back to whatever order a database handed
them over in, and two peers reading two snapshots of the same history order them
differently. There is deliberately **no record id**: ordering does not need one,
and a consumer that wants a handle can hash the canonical line (§7).

**One line is not an event: the header** (§5.2). It carries `t = 0` so it sorts
ahead of everything, and it is the only record in the vocabulary that describes
the stream rather than something that happened.

## 2. Kinds

| `k` | Is | Fields |
|---|---|---|
| `cgm` | A sensor reading | `mgdl`, `trend`, `src` |
| `bolus` | Insulin delivered as a bolus | `u`, `type`, `basal` |
| `carb` | Carbohydrate entered | `g`, `dur` |
| `tbr` | Temporary basal rate | `rate`, `abs`, `dur`, `type` |
| `extbolus` | Extended bolus | `u`, `dur` |
| `event` | Site change, sensor change, note, finger-stick | `type`, `dur`, `note`, `mgdl` |
| `target` | Temporary target | `lo`, `hi`, `dur`, `why` |
| `profile` | Profile switch, with the blocks | `name`, `pct`, `shift`, `dur`, `basal`, `isf`, `ic`, `target` |
| `meta` | **The stream header, not an event** (§5.2) | `spec`, `epoch`, `unit` |

**`tdd` was in this table and is not in v1.** Total daily dose is derived — a
consumer holding boluses and basals computes it — and derived data in a stream is
a second source of truth that will eventually disagree with the first. The
measurement settled it: across five real snapshots AAPS had written **0, 0, 0, 1
and 19 rows**. A consumer cannot rely on a field that is absent three times in
five, so it has to compute the value anyway, and then it has two. Removing a kind
before the freeze is free; after it costs a compatibility window.

**`extbolus` stays and has never been exercised.** It is zero on all five
snapshots, because this pump and configuration do not use extended boluses. That
is an argument about one person's setup, not about the vocabulary — but the first
real extended bolus to reach a consumer will be the first one this emitter has
ever produced, and it should be treated that way.

**Units are fixed and never carried per-record.** `mgdl` is mg/dL, `u` is units
of insulin, `g` is grams, **`dur` is milliseconds**, `rate` is U/h when `abs` is
true and percent otherwise.

**Milliseconds, not minutes, and not by preference.** It is what AAPS stores, it
matches `t`, and it is the only choice that stays an integer: real durations
include `36690` and `1195731` ms, which are 0.61 and 19.93 minutes. A unit that
forces the emitter to round is a unit that loses information the device had. A stream that lets each record declare its own units is a
stream where one mis-set flag becomes a dosing-scale error in somebody's
analysis.

**That rule cost the `profile` record its `unit` field, and it was not free.**
AAPS stores `glucoseValues` and `temporaryTargets` in mg/dL always, but it
stores *profile blocks in whichever unit the user set*. On this project's own
reference snapshot that put `target.lo = 160.2` (mg/dL) and
`profile.target[].lowTarget = 5` (mmol/L) in the same stream, meaning the same
kind of quantity **a factor of 18.0182 apart**, distinguishable only by reading
a flag on one of them. So the conversion happens **at the emit boundary**: ISF
and target blocks are normalised to mg/dL using AAPS's own constant — the one
the loop dosed on — and the record carries no unit.

The one case that cannot be resolved is a `glucoseUnit` the emitter does not
recognise. Those blocks are passed through untouched **and** carry `unit`, so a
consumer meets an explicit *"this one is not normalised"* rather than a
plausible wrong number. `--stats` names them.

**`basal` (U/h) and `ic` (g/U) carry no glucose unit and are never scaled.**

**Profile blocks carry their own `duration`, also in milliseconds** — a
whole-day block is `86400000`. And **`shift` is unverified**: it is zero on every
profile in every snapshot checked, so nothing here establishes its unit. Anyone
emitting a non-zero `shift` should measure it first rather than trust this table.

**`profile` carries the blocks, not just the name**, and carries them as
**parsed arrays, not as strings**. AAPS stores each block column as a JSON
string; passing it through verbatim would hand every consumer JSON inside JSON
and charge for the escaping on every profile record. Anything that will not
parse is carried verbatim rather than dropped — a profile without its blocks is
uninterpretable, and losing one silently is worse than handing on a string
somebody has to look at.

Without basal rates, ISF, IC and targets by time of day, a consumer cannot say
what the loop was *trying* to do, and the insulin records become
uninterpretable.

**`event.note` is free text a person typed.** It is the field most likely to name
a third party, and anything that narrows a grant should narrow this first.

## 3. What is removed, and why

A dump of the AAPS database is not a person's history. It is the history plus its
own edit log. Three filters, applied at the emit boundary because a consumer that
takes the tables at face value over-counts and biases every model fitted on them.

### 3.1 Version rows — `referenceId IS NOT NULL`

AAPS keeps history in-band: a modified record becomes a new row whose
`referenceId` points at the one it supersedes. **These are not events.**

**Measured, four snapshots of one real loop.** `glucoseValues` holds close to two
rows per reading, and the current rows account for *every distinct timestamp*:

| snapshot | rows | current | distinct timestamps |
|---|---|---|---|
| snap_chk | 23,948 | 11,974 | 11,974 |
| f | 31,532 | 16,400 | 16,400 |
| snap_0721 | 10,498 | 5,435 | 5,435 |
| androidaps2 | 1,708 | 854 | 854 |

And after filtering, `rows == distinct timestamps == distinct 5-minute buckets`,
ratio **1.000** on every snapshot — so there is no same-bucket duplication
either, only version history.

A version row is written when NSClient stamps a `nightscoutId`; the pair differs
only in `version`, `referenceId` and that id.

> **Correction to an earlier belief.** This doubling was previously attributed to
> an xDrip double-broadcast, at ~1.85x. It is not — it is version history, on
> every snapshot checked. Once `referenceId IS NULL` is applied there are **zero
> duplicate CGM timestamps**, and the rate falls to **272.4/day — 95% of the 288**
> a 5-minute sensor can produce, which is ordinary sensor uptime.
>
> **A second correction, to the correction.** That rate was first reported here
> as 246.6/day, which divided the CGM count by the span of the *whole stream*.
> The stream starts 4.6 days before the sensor produced anything, so the figure
> counted days on which no CGM existed. A rate is only a rate over the period the
> thing was running.

### 3.2 Retracted rows — `isValid = 0`

The user or the loop withdrew it. It stays in the table.

### 3.3 CGM outside its bucket

One reading per five minutes, **keeping the first**. Not the last, and not the
mean: the first is the reading the loop actually saw and acted on, and averaging
would invent a value no device reported and no dose was based on.

On the snapshots above this drops **nothing** — §3.1 already removed the
doubling. It is kept as defence in depth, because a genuinely double-broadcasting
source would otherwise reach consumers unnoticed, and the check is free.

## 4. What is excluded

`deviceStatus` and `apsResults` — **two thirds of the database**, 30 MB of the 45
MB in the reference snapshot. Loop telemetry: Nightscout plumbing and algorithm
debug. They triple the payload for no clinical content and are the tables most
likely to hold something nobody meant to share.

There is no flag to include them. There was one — `--include-telemetry` — and it
never did anything but print a warning claiming it had. Loop telemetry is not in
the vocabulary in §2 at all, so a flag that injected it would emit records
outside the contract this document exists to fix; dumping it is a job for
`sqlite3`.

## 5. Epochs, and the header

### 5.1 An epoch is a day, cut at a fixed offset

**The unit of scoping.** Grants think in epochs, and a consumer buckets by them.

```
epoch = floor((t + offset) / 86400000)
```

**Why not plain UTC, which v1 and v2 used.** The argument for UTC was that an
epoch must have the same identity on every device — a local-midnight boundary is
ambiguous across travel and DST, and two peers disagreeing about which epoch a
record belongs to is a correctness problem in a replicated store. **All of that
is still true, and a fixed offset satisfies it**: it is a constant, so a record
maps to the same epoch wherever and whenever it was written.

What UTC got wrong was the **phase**. Measured on this project's own data, which
is UTC+12:

```
epoch 20661:  Mon 27 Jul 12:00  →  Tue 28 Jul 12:00  local
```

A UTC epoch runs local noon to local noon, so **one local day is split 12 hours
either side of two epochs** — "share yesterday" shares two half-days, and a
windowed grant is half a day out at both ends. Worse, when revocation waited for
the epoch boundary the worst case, nearly a full day retained, landed **just
after local noon, in the middle of the waking day**. Shifted to local midnight
the worst case lands while the subject is asleep.

**This is not local time.** The offset is a fixed constant recorded once, so it
does not follow DST and does not move when the subject travels. `tools/canon.py`
derives it from the mode of AAPS's own `utcOffset` column — where someone lives,
rather than where they happened to be when a snapshot was taken.

**An epoch is not a key.** Revocation no longer waits for the boundary: the
sealing layer cuts a new **segment** on withdrawal, so what a reader keeps is
bounded by *when they were revoked*, not by the epoch. An epoch may hold several
segments; it is still the unit grants and consumers speak in. That is a property
of the sealing layer rather than of this document, and it is why epoch length is
now a question of cost and scoping rather than of safety.

**Measured, on the reference snapshot:** 47 epochs, 245 key wraps for five
readers — about **24 KB of key records beside 1.66 MB of data**, which is
179 KB/year against the 180 KB/year feasibility.md §7.2 predicted.

### 5.2 The header declares what a consumer cannot infer

The first line of an encoded stream:

```json
{"epoch":"offset-day","k":"meta","offset":43200000,"spec":3,"t":0,"unit":"mgdl"}
```

- **`spec`** — the version of this document the stream conforms to.
- **`epoch`** — how epochs are cut, so the sealing layer and a reader agree
  without a side channel.
- **`offset`** — the phase they are cut at, in milliseconds. A consumer cannot
  infer this, and the same records cut at a different phase are a **different set
  of days**: every daily figure computed from them would be quietly wrong.
- **`unit`** — that **every** glucose quantity in the stream is mg/dL, *including
  the profile blocks*, which AAPS itself stores in the user's own unit.

**The third one is why this exists.** After §2's normalisation a normalised
stream and an un-normalised one are indistinguishable by inspection — the numbers
are simply eighteen times apart — and a consumer that guesses wrong is wrong by
a dosing scale factor. A stream that carries insulin should say what it is.

## 6. Encoding

**NDJSON is the POC format, not the wire format.** It was chosen so a stream can
be eyeballed, diffed and piped.

The wire format wants delta-and-varint records sealed per epoch. Until it exists,
gzip stands in for it as an upper bound:

```
1.66 MB ndjson → 0.19 MB gzip   over 48.5 days
12.4 MB/year   → 1.5 MB/year    projected
```

Measured on the reference snapshot: 19,128 records, 1,655,863 bytes of NDJSON,
194,518 gzipped. The 48.5 days is the span of the **whole stream**; the CGM in it
covers 44.0 of those days (§3.3).

**Treat the record shape as the contract and the encoding as replaceable.**

## 7. Still open, after v1

**Settled**, and recorded here so they are not re-opened by accident: the schema
version (§5.2, in-band), `tdd` (§2, removed), record ordering (§1,
canonical-encoding tie-break, no id), durations in milliseconds (§2, v2), and
epoch phase (§5.1, a fixed offset, v3).

What remains:

1. **A stable record id, for the things ordering does not cover.** Citation —
   *"this finding rests on these records"* — and any future notion of retraction
   both want one. A consumer that needs a handle today can hash the canonical
   line, which is free and requires no agreement; what does not exist is a
   *stable* id that survives the record being re-encoded under a v2. Deferred
   deliberately: adding a field is a version bump, and nothing needs it yet.
2. **What a v2 is allowed to do.** The version number is now declared, and
   nothing says what a consumer should do when it meets a number it does not
   know. Refusing is safe and useless; proceeding is useful and unsafe. This
   wants deciding before there is a second implementation, not after.
3. **How a correction reaches an epoch that is already sealed.** This document's
   filters were derived from *snapshots*, which show only the final state. The
   AAPS sync queue shows the edits: `getNextModifiedOrNewAfter` is
   `SELECT * FROM t WHERE id > :id ORDER BY id ASC LIMIT 1` — no filter on
   `isValid` or `referenceId` — and when it lands on a version row it resolves to
   the **current** record and emits that. So a live emitter receives the same
   logical record again every time it is edited, and §7.4 says a published epoch
   cannot be unpublished.

   **Measured on the reference snapshot, and it is small.** Of 12,876 version
   rows, **12,874 are semantically identical** to the record they supersede —
   NSClient stamping a `nightscoutId`, no clinical change. **Two are real edits**,
   both carbs, over 48.5 days.

   | | |
   |---|---|
   | Re-emissions carrying no change | **12,874** — deduplicate by canonical content, no vocabulary needed |
   | Genuine edits | **2 in 48.5 days**, roughly 15 a year |
   | Retractions (`isValid = 0`) | 3 in the same window |

   So the emitter must deduplicate by canonical content regardless, and that
   settles 99.98% of it. What is left is **an edit or retraction arriving after
   its epoch was sealed**, perhaps fifteen times a year. A retraction of
   published data is the "delete my data" that §7.4 says is not available; the
   most that can be offered is a correction consumers apply.

   **Deliberately not designed yet.** Fifteen events a year is too few to guess a
   mechanism from, and an `amend` kind would push exactly the edit-log-application
   that §3 refuses onto consumers. The plugin should count them first. The kind
   name `amend` is reserved so a v2 can take it.

4. **`event.note` and the grant that narrows it.** The note is free text a person
   typed, and §2 already says it is the field most likely to name a third party.
   Nothing yet expresses *"this grant covers the stream without the notes"*,
   which is the narrowing a person is most likely to want first.
