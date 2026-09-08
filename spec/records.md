# The canonical record stream

**Status:** draft, and the thing to freeze first. Every later stage encodes
against this; changing it once a peer exists costs a compatibility window.

Produced by [`tools/canon.py`](../tools/canon.py). Consumed by the sealing layer,
the AAPS plugin and the commons gateway.

---

## 1. What a record is

One event, at one instant, as the device actually recorded it. Not a database
row — see §3.

```json
{"t":1782938503230,"k":"cgm","mgdl":163.0,"src":"Dexcom G6","trend":"FLAT"}
{"t":1782938700000,"k":"bolus","u":1.25,"type":"NORMAL","basal":false}
{"t":1782938700000,"k":"carb","g":30.0,"dur":0}
```

- **`t`** — epoch milliseconds, UTC. The sort key and the only mandatory field
  besides `k`. Local time is a rendering concern; a stream that carries local
  time cannot be merged across a timezone change, and people travel.
- **`k`** — the kind. A closed, versioned vocabulary (§2).
- Everything else is per-kind and **optional**: a missing field means the device
  did not report one, which is different from zero and must stay different.

**Ordering** is by `(t, k)`. Ties are real — a bolus and its carbs share a
timestamp — so `k` breaks them deterministically rather than leaving two peers to
disagree about order.

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
| `profile` | Profile switch, with the blocks | `name`, `pct`, `shift`, `dur`, `basal`, `isf`, `ic`, `target`, `unit` |
| `tdd` | Total daily dose | `basal`, `bolus`, `total`, `g` |

**Units are fixed and never carried per-record.** `mgdl` is mg/dL, `u` is units
of insulin, `g` is grams, `dur` is minutes, `rate` is U/h when `abs` is true and
percent otherwise. A stream that lets each record declare its own units is a
stream where one mis-set flag becomes a tenfold dosing error in somebody's
analysis.

**`profile` carries the blocks, not just the name.** Without basal rates, ISF, IC
and targets by time of day, a consumer cannot say what the loop was *trying* to
do, and the insulin records become uninterpretable.

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

A version row is written when NSClient stamps a `nightscoutId`; the pair differs
only in `version`, `referenceId` and that id.

> **Correction to an earlier belief.** This doubling was previously attributed to
> an xDrip double-broadcast, at ~1.85x. It is not — it is version history, on
> every snapshot checked. Once `referenceId IS NULL` is applied there are **zero
> duplicate CGM timestamps**, and the rate falls to a plausible 246.6/day against
> the 288 a 5-minute sensor can produce.

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

`--include-telemetry` exists for debugging a loop, not for sharing a history.

## 5. Encoding

**NDJSON is the POC format, not the wire format.** It was chosen so a stream can
be eyeballed, diffed and piped.

The wire format wants delta-and-varint records sealed per epoch. Until it exists,
gzip stands in for it as an upper bound:

```
1.66 MB ndjson → 0.19 MB gzip   over 48.5 days
12.5 MB/year   → 1.5 MB/year    projected
```

**Treat the record shape as the contract and the encoding as replaceable.**

## 6. Still to decide

1. **Epoch boundaries.** Where a day starts, and in whose timezone. UTC is
   simplest and puts the boundary in the middle of the night for nobody in
   particular; local time makes epochs ambiguous across travel.
2. **A record id.** Currently a record is identified by `(t, k)`, which is
   unique in every snapshot checked but is not guaranteed to be — two carb
   entries in the same millisecond are possible, if unlikely.
3. **Schema version in-band.** Nothing currently says which version of this
   document a stream conforms to. It should, before there is a second
   implementation.
4. **Whether `tdd` belongs at all.** It is derived — a consumer holding boluses
   and basals can compute it — and derived data in a stream is a second source
   of truth that will eventually disagree with the first.
