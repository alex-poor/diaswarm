#!/usr/bin/env python3
"""
Canonicalise an AAPS database into the swarm record stream.

This is stage 10.1 of docs/feasibility.md — the piece no framework provides, and
the one thing that has to be right before any of them is chosen. It is
deliberately framework-neutral: it reads a SQLite snapshot and writes records.
No network, no keys, no peers.

WHY THIS EXISTS AT ALL, rather than "just sync the tables".

  A dump of the AAPS database is not the person's history. It is the history
  plus its own edit log plus its uploader's retries, and every consumer that
  has ever taken one at face value has silently over-counted. Three specific
  hazards, all of them measured on real data (see --stats):

    1. VERSION ROWS.  AAPS keeps history in-band: a modified record is a new
       row whose `referenceId` points at the one it supersedes. Those rows are
       not events. Counting them counts the same bolus twice.

    2. INVALIDATED ROWS.  `isValid = 0` means the user or the loop retracted
       it. It stays in the table.

    3. CGM OUTSIDE ITS BUCKET.  One reading per five minutes, keeping the
       first — the one the loop actually saw and acted on.

  CORRECTION, 2026-09-08.  Hazard 3 was previously written here as an xDrip
  double-broadcast at ~1.85x: 544 readings a day against the 288 a 5-minute
  sensor can physically produce. Measured across four snapshots, it is not.
  It is hazard 1 — version history — on every one of them. Once
  `referenceId IS NULL` is applied there are zero duplicate CGM timestamps and
  zero same-bucket duplicates, at 272.4 readings/day — 95% of the 288 a 5-minute
  sensor can produce, which is ordinary sensor uptime. The debounce
  stays as defence in depth, because a genuinely double-broadcasting source
  would otherwise reach consumers unnoticed and the check is free.
  See spec/records.md §3.1 for the four-snapshot table.

  Ship the tables raw and a downstream model sees roughly 2x the real insulin
  and carbs, which biases every fit in the same direction. Canonicalisation is
  therefore part of the protocol, not a consumer's problem.

WHAT IS EXCLUDED, AND WHY IT MATTERS MORE THAN IT SOUNDS.

  `deviceStatus` and `apsResults` are two thirds of the file — 30 MB of the 45
  MB in the reference snapshot — and they are loop telemetry: Nightscout
  plumbing and algorithm debug. Carrying them triples the payload for no
  clinical content, and they are the tables most likely to contain something
  nobody meant to share. They are not in the record vocabulary at all — the
  canonical stream is a closed set of kinds (spec/records.md §2) — so dumping
  loop telemetry is a job for `sqlite3`, not for this tool.

Usage:
    tools/canon.py SNAPSHOT.db --stats
    tools/canon.py SNAPSHOT.db -o out/stream.ndjson
"""

from __future__ import annotations

import argparse
import gzip
import json
import sqlite3
import sys
from collections import Counter
from pathlib import Path

# A CGM sensor produces one reading per five minutes. Anything finer is a
# duplicate broadcast, not a measurement, so the bucket is a property of the
# hardware rather than a tuning knob.
CGM_BUCKET_MS = 5 * 60 * 1000

# The version of spec/records.md this emitter conforms to. Declared in the
# stream's own header, because after glucose normalisation a consumer cannot
# otherwise tell a normalised stream from an un-normalised one by looking at it.
SPEC_VERSION = 1

# One epoch, one content key (feasibility.md §7.2). UTC so that an epoch has the
# same identity on every device: a local-midnight boundary is ambiguous across
# travel and DST, and two peers disagreeing about which epoch a record belongs
# to is a correctness problem in a replicated store, not a cosmetic one. The
# cost is real and named — away from UTC the boundary falls inside the waking
# day, so "they keep the rest of the epoch" is harder to say plainly.
EPOCH_MS = 24 * 60 * 60 * 1000
EPOCH_BASIS = "utc-day"

# AAPS's own constant (Constants.MMOLL_TO_MGDL), not the textbook 18. Profile
# blocks are stored in whichever unit the user set, so converting with the same
# constant the loop used is what reproduces the numbers it actually dosed on.
MMOLL_TO_MGDL = 18.0182

# Profiles whose glucoseUnit was neither MGDL nor MMOL, collected for the
# report. Their blocks cannot be normalised, so they carry `unit` and the
# ambiguity is made visible rather than guessed at.
UNKNOWN_UNITS: list[str] = []

# Every table the canonical stream is built from. One list, because extract()
# and report() drifting apart is how a table quietly stops being counted.
TABLES = (
    "glucoseValues", "boluses", "carbs", "temporaryBasals", "therapyEvents",
    "temporaryTargets", "extendedBoluses", "profileSwitches",
)

# Tables _rows() could not read, collected for the report. A schema mismatch
# that only prints to stderr is a kind silently missing from the output.
SKIPPED: list[tuple[str, str]] = []


class Kind:
    """Record kinds in the canonical stream. Stable wire names, kept short."""

    CGM = "cgm"
    BOLUS = "bolus"
    CARB = "carb"
    TBR = "tbr"
    EXT_BOLUS = "extbolus"
    EVENT = "event"
    TARGET = "target"
    PROFILE = "profile"

    # Not an event: the stream's own header. See header().
    META = "meta"


def _rows(db: sqlite3.Connection, table: str, columns: str) -> list[sqlite3.Row]:
    """Select the valid, non-superseded rows of one table, oldest first.

    The two WHERE clauses are the whole of hazards 1 and 2 above, and they are
    applied here rather than per-kind so that no extractor can forget them.

    A table absent from the snapshot yields nothing rather than raising: AAPS
    schemas change across releases, and a canonicaliser that dies on an older
    database is useless for exactly the longitudinal history this exists to
    move.
    """
    try:
        cur = db.execute(
            f"SELECT {columns} FROM {table} "  # noqa: S608 - table names are constants above
            "WHERE isValid = 1 AND referenceId IS NULL "
            "ORDER BY timestamp"
        )
        return cur.fetchall()
    except sqlite3.OperationalError as e:
        SKIPPED.append((table, str(e)))
        print(f"  skip {table}: {e}", file=sys.stderr)
        return []


def _dropped(db: sqlite3.Connection, table: str) -> tuple[int, int]:
    """How many rows each filter removed, for the stats report.

    The two counts are made disjoint — a retracted row that is also a version
    row is counted once, as a version row. They are printed as a breakdown of
    one total, and a breakdown whose parts overlap is a wrong total.
    """
    try:
        invalid = db.execute(
            f"SELECT count(*) FROM {table} "  # noqa: S608
            "WHERE isValid = 0 AND referenceId IS NULL"
        ).fetchone()[0]
        versioned = db.execute(
            f"SELECT count(*) FROM {table} WHERE referenceId IS NOT NULL"  # noqa: S608
        ).fetchone()[0]
        return invalid, versioned
    except sqlite3.OperationalError:
        return 0, 0


def _rec(**fields) -> dict:
    """Build a record, dropping fields the device did not report.

    A missing field and a zero are different facts — a carb entry with no
    duration is not a carb entry with a duration of zero — and §1 of the spec
    makes that distinction load-bearing. Emitting an explicit null for every
    unreported field erases it, and pays for the null on every record of every
    day for a decade.
    """
    return {k: v for k, v in fields.items() if v is not None}


def _blocks(raw, scale: float = 1.0, fields: tuple[str, ...] = ()):
    """Parse a profile block column, which AAPS stores as a JSON string.

    Passed through as a string it would reach consumers as JSON inside JSON,
    forcing a second parse and paying for the escaping. Anything unparseable is
    carried verbatim rather than dropped: a profile without its blocks is
    uninterpretable, and losing one silently is worse than handing a consumer a
    string it has to look at.

    `scale` and `fields` normalise the glucose-bearing blocks to mg/dL — see
    _profile_scale(). Basal (U/h) and IC (g/U) carry no glucose unit and are
    never scaled.
    """
    if raw is None:
        return None
    try:
        blocks = json.loads(raw)
    except (json.JSONDecodeError, TypeError):
        return raw
    if scale == 1.0 or not isinstance(blocks, list):
        return blocks
    for b in blocks:
        if not isinstance(b, dict):
            continue
        for f in fields:
            if isinstance(b.get(f), (int, float)):
                b[f] = round(b[f] * scale, 1)
    return blocks


def _profile_scale(unit) -> tuple[float, str | None]:
    """How to get this profile's blocks into mg/dL, and what to admit if we can't.

    THE HAZARD THIS EXISTS FOR. `glucoseValues.value` and
    `temporaryTargets.{low,high}Target` are stored in mg/dL always, but profile
    blocks are stored in whatever unit the user set. Emitting both untouched
    puts a target of 108 (mg/dL) and a target of 5 (mmol/L) in the same stream
    meaning nearly the same thing — a factor of eighteen apart — which is
    exactly the failure §2 of the spec forbids: one mis-read unit flag becoming
    a dosing-scale error in somebody's analysis.

    So the conversion happens here, at the emit boundary, and the record carries
    no unit. An unrecognised unit is the one case that cannot be resolved: those
    blocks are emitted untouched WITH a `unit` field, so a consumer sees an
    explicit "this one is not normalised" rather than a plausible wrong number.
    """
    u = (unit or "").strip().upper()
    if u in ("MGDL", "MG/DL", "MGDL/L"):
        return 1.0, None
    if u in ("MMOL", "MMOLL", "MMOL/L"):
        return MMOLL_TO_MGDL, None
    UNKNOWN_UNITS.append(unit if unit is not None else "<null>")
    return 1.0, unit


def epoch_of(t: int) -> int:
    """Which epoch a timestamp falls in. The unit of key custody, not of storage."""
    return t // EPOCH_MS


def header() -> dict:
    """The first line of an encoded stream. The only record that is not an event.

    It carries `t` and `k` like everything else, and `t` is 0 so that it sorts
    ahead of every real record if anything ever re-sorts the stream. What it
    declares is the three things a consumer cannot work out by looking:

      spec   which version of spec/records.md this conforms to
      epoch  how epochs are cut, so the sealing layer and a reader agree
      unit   that every glucose quantity in the stream is mg/dL, INCLUDING the
             profile blocks, which AAPS itself stores in the user's own unit
    """
    return {
        "t": 0,
        "k": Kind.META,
        "spec": SPEC_VERSION,
        "epoch": EPOCH_BASIS,
        "unit": "mgdl",
    }


def _canon_json(record: dict) -> str:
    """The one canonical serialisation of a record. Also the tie-break in sort."""
    return json.dumps(record, separators=(",", ":"), sort_keys=True)


def _round(value, digits: int):
    """Round, preserving None.

    Values are rounded to the precision the device actually has. A pump that
    delivers in 0.01 U steps has no business emitting a float with seventeen
    significant figures, and the extra digits cost real bytes once there is one
    record every four minutes for a decade.
    """
    return None if value is None else round(value, digits)


def extract(db: sqlite3.Connection) -> list[dict]:
    """Build the canonical record list from an AAPS database."""
    out: list[dict] = []

    for r in _rows(db, "glucoseValues", "timestamp, value, trendArrow, sourceSensor"):
        out.append(
            _rec(
                t=r["timestamp"],
                k=Kind.CGM,
                mgdl=_round(r["value"], 1),
                trend=r["trendArrow"],
                src=r["sourceSensor"],
            )
        )

    for r in _rows(db, "boluses", "timestamp, amount, type, isBasalInsulin"):
        out.append(
            _rec(
                t=r["timestamp"],
                k=Kind.BOLUS,
                u=_round(r["amount"], 3),
                type=r["type"],
                basal=bool(r["isBasalInsulin"]),
            )
        )

    for r in _rows(db, "carbs", "timestamp, amount, duration"):
        out.append(
            _rec(
                t=r["timestamp"],
                k=Kind.CARB,
                g=_round(r["amount"], 1),
                dur=r["duration"],
            )
        )

    for r in _rows(db, "temporaryBasals", "timestamp, type, isAbsolute, rate, duration"):
        out.append(
            _rec(
                t=r["timestamp"],
                k=Kind.TBR,
                rate=_round(r["rate"], 3),
                abs=bool(r["isAbsolute"]),
                dur=r["duration"],
                type=r["type"],
            )
        )

    for r in _rows(db, "extendedBoluses", "timestamp, amount, duration"):
        out.append(
            _rec(
                t=r["timestamp"],
                k=Kind.EXT_BOLUS,
                u=_round(r["amount"], 3),
                dur=r["duration"],
            )
        )

    # `note` is free text a person typed. It is carried because a site change or
    # an illness note is often the only explanation for a week of odd data — and
    # it is the field most likely to name a third party, so anything that
    # narrows a grant should narrow this first.
    for r in _rows(db, "therapyEvents", "timestamp, duration, type, note, glucose"):
        out.append(
            _rec(
                t=r["timestamp"],
                k=Kind.EVENT,
                type=r["type"],
                dur=r["duration"],
                note=r["note"] or None,
                mgdl=_round(r["glucose"], 1),
            )
        )

    for r in _rows(db, "temporaryTargets", "timestamp, reason, highTarget, lowTarget, duration"):
        out.append(
            _rec(
                t=r["timestamp"],
                k=Kind.TARGET,
                lo=_round(r["lowTarget"], 1),
                hi=_round(r["highTarget"], 1),
                dur=r["duration"],
                why=r["reason"],
            )
        )

    # Blocks are the profile itself (basal rates, ISF, IC, targets by time of
    # day). Without them a consumer cannot say what the loop was *trying* to do,
    # which makes the insulin records uninterpretable.
    # The profile's glucose unit decides the scale; it is applied here and the
    # unit is not carried onward. See _profile_scale().
    for r in _rows(
        db,
        "profileSwitches",
        "timestamp, profileName, percentage, timeshift, duration, "
        "basalBlocks, isfBlocks, icBlocks, targetBlocks, glucoseUnit",
    ):
        scale, unresolved = _profile_scale(r["glucoseUnit"])
        out.append(
            _rec(
                t=r["timestamp"],
                k=Kind.PROFILE,
                name=r["profileName"],
                pct=r["percentage"],
                shift=r["timeshift"],
                dur=r["duration"],
                basal=_blocks(r["basalBlocks"]),
                isf=_blocks(r["isfBlocks"], scale, ("amount",)),
                ic=_blocks(r["icBlocks"]),
                target=_blocks(r["targetBlocks"], scale, ("lowTarget", "highTarget")),
                unit=unresolved,
            )
        )

    # (t, k) is the spec's order, and it is not guaranteed unique — two carb
    # entries in one millisecond are possible. Ties therefore break on the
    # record's own canonical encoding: every implementation can compute it and
    # none has to agree in advance. Without it, equal-(t, k) records fall back
    # to whatever order the database handed them over in, and two peers reading
    # two snapshots of the same history can order them differently.
    out.sort(key=lambda rec: (rec["t"], rec["k"], _canon_json(rec)))
    return out


def debounce_cgm(records: list[dict]) -> tuple[list[dict], int]:
    """Collapse CGM readings to one per five-minute bucket.

    KEEPS THE FIRST of a bucket, not the last and not the mean. The first is the
    reading the loop actually saw and acted on; a later duplicate of the same
    measurement arrived after the decision was made. Averaging would invent a
    value that no device ever reported and that no dose was ever based on.

    Non-CGM records pass through untouched — a bolus and a carb entry a minute
    apart are two events, not a duplicate.
    """
    seen: set[int] = set()
    kept: list[dict] = []
    dropped = 0

    for rec in records:
        if rec["k"] != Kind.CGM:
            kept.append(rec)
            continue
        bucket = rec["t"] // CGM_BUCKET_MS
        if bucket in seen:
            dropped += 1
            continue
        seen.add(bucket)
        kept.append(rec)

    return kept, dropped


def encode(records: list[dict]) -> bytes:
    """Serialise to NDJSON.

    NDJSON is the POC format, chosen so a stream can be eyeballed, diffed and
    piped. It is NOT the wire format — §7 of the feasibility doc wants
    delta-and-varint records sealed per epoch, and the gzip figure in --stats
    stands in for that until it exists. Anything downstream should treat the
    record SHAPE as the contract and the encoding as replaceable.
    """
    return b"".join(_canon_json(r).encode() + b"\n" for r in [header(), *records])


def report(db: sqlite3.Connection, records: list[dict], cgm_dropped: int, blob: bytes) -> None:
    """Print what was kept, what was dropped, and what it projects to."""
    kinds = Counter(r["k"] for r in records)
    span_ms = (records[-1]["t"] - records[0]["t"]) if records else 0
    days = span_ms / 86_400_000 or 1

    print("\n  kept", file=sys.stderr)
    for kind, n in sorted(kinds.items(), key=lambda kv: -kv[1]):
        print(f"    {kind:<10} {n:>8,}", file=sys.stderr)
    print(f"    {'TOTAL':<10} {len(records):>8,}", file=sys.stderr)

    print("\n  dropped", file=sys.stderr)
    tot_inv = tot_ver = 0
    for t in TABLES:
        inv, ver = _dropped(db, t)
        tot_inv += inv
        tot_ver += ver
    print(f"    {'invalid':<10} {tot_inv:>8,}   isValid = 0", file=sys.stderr)
    print(f"    {'version':<10} {tot_ver:>8,}   referenceId IS NOT NULL", file=sys.stderr)
    print(f"    {'cgm dup':<10} {cgm_dropped:>8,}   second+ reading in a 5-min bucket", file=sys.stderr)

    # Per DAY OF CGM, not per day of stream. The stream starts at the first
    # record of any kind, and on the reference snapshot that is 4.6 days before
    # the sensor produced anything — dividing by the whole span reports a rate
    # for days on which no CGM existed, and understates the real one by 10%.
    cgm = kinds.get(Kind.CGM, 0)
    if cgm:
        cgm_t = [r["t"] for r in records if r["k"] == Kind.CGM]
        cgm_days = (cgm_t[-1] - cgm_t[0]) / 86_400_000
        per_day = cgm / cgm_days if cgm_days >= 1 else 0
        if cgm_days >= 1:
            print(
                f"\n  cgm     {per_day:>6.1f}/day after debounce over {cgm_days:.1f} "
                f"days of CGM\n          ({per_day / 288 * 100:.0f}% of the 288 a "
                f"5-minute sensor can produce)",
                file=sys.stderr,
            )
        else:
            # A rate from under a day of data is noise wearing a decimal point.
            print(
                f"\n  cgm     {cgm:,} readings over {cgm_days * 24:.1f} hours — too "
                f"short a span to quote a daily rate",
                file=sys.stderr,
            )
        if cgm_dropped:
            # Nothing measured has ever reached this branch — see the module
            # docstring. If it fires, the source really is broadcasting twice.
            ratio = (cgm + cgm_dropped) / cgm
            print(
                f"          raw was {ratio:.2f}x that — a source broadcasting "
                f"more than once per bucket",
                file=sys.stderr,
            )

    if UNKNOWN_UNITS:
        print(
            f"\n  UNRECOGNISED GLUCOSE UNIT on {len(UNKNOWN_UNITS)} profile(s): "
            f"{', '.join(sorted(set(UNKNOWN_UNITS)))}\n"
            "  Their blocks are NOT normalised to mg/dL and carry `unit` to say so.",
            file=sys.stderr,
        )

    if SKIPPED:
        print(
            "\n  NOT READ — these tables are missing from the stream, and a kind\n"
            "  absent from `kept` above may be absent for this reason rather than\n"
            "  because nothing happened:",
            file=sys.stderr,
        )
        for table, err in SKIPPED:
            print(f"    {table:<20} {err}", file=sys.stderr)

    if records:
        epochs = epoch_of(records[-1]["t"]) - epoch_of(records[0]["t"]) + 1
        print(
            f"\n  epochs  {epochs:>7,}   UTC days — {epochs * 5:,} key wraps for five "
            f"readers,\n          about {epochs * 5 * 100 / 1024:.0f} KB of key records "
            f"beside {len(blob) / 1e6:.2f} MB of data",
            file=sys.stderr,
        )

    gz = gzip.compress(blob, 9)
    print(
        f"\n  size    {len(blob) / 1e6:>7.2f} MB ndjson"
        f"   {len(gz) / 1e6:>6.2f} MB gzip"
        f"   over {days:.1f} days",
        file=sys.stderr,
    )
    if days >= 1:
        print(
            f"  year    {len(blob) / 1e6 * 365 / days:>7.1f} MB ndjson"
            f"   {len(gz) / 1e6 * 365 / days:>6.1f} MB gzip"
            f"   projected",
            file=sys.stderr,
        )
    else:
        # Projecting a year from an hour is how §4's original estimate went
        # wrong in the first place. Refuse rather than print it.
        print(
            "  year    not projected — the stream spans less than a day",
            file=sys.stderr,
        )
    print(
        "\n  The gzip column is the number docs/feasibility.md §4 rests on.\n"
        "  A real delta-and-varint encoding should beat it.",
        file=sys.stderr,
    )


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[1])
    ap.add_argument("db", type=Path, help="AAPS SQLite snapshot (copy the -wal too)")
    ap.add_argument("-o", "--out", type=Path, help="write NDJSON here (default: stdout)")
    ap.add_argument("--stats", action="store_true", help="report to stderr and write nothing")
    args = ap.parse_args()

    if not args.db.exists():
        print(f"no such database: {args.db}", file=sys.stderr)
        return 1

    # Read-only, so a live snapshot cannot be modified by inspecting it.
    db = sqlite3.connect(f"file:{args.db}?mode=ro", uri=True)
    db.row_factory = sqlite3.Row

    print(f"  read {args.db}", file=sys.stderr)
    records = extract(db)
    records, cgm_dropped = debounce_cgm(records)
    blob = encode(records)

    if args.stats or not args.out:
        report(db, records, cgm_dropped, blob)

    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_bytes(blob)
        print(f"\n  wrote {args.out} ({len(blob):,} bytes)", file=sys.stderr)
    elif not args.stats:
        sys.stdout.buffer.write(blob)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
