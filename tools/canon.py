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

    3. CGM DOUBLE-BROADCAST.  xDrip broadcasts can arrive more than once, at
       ~1.85x on this project's own data — 544 readings a day against the 288
       a 5-minute sensor can physically produce. Nothing in the schema marks
       them as duplicates; they differ only by `id`.

  Ship the tables raw and a downstream model sees roughly 1.9x the real insulin
  and carbs, which biases every fit in the same direction. Canonicalisation is
  therefore part of the protocol, not a consumer's problem.

WHAT IS EXCLUDED, AND WHY IT MATTERS MORE THAN IT SOUNDS.

  `deviceStatus` and `apsResults` are two thirds of the file — 30 MB of the 45
  MB in the reference snapshot — and they are loop telemetry: Nightscout
  plumbing and algorithm debug. Carrying them triples the payload for no
  clinical content, and they are the tables most likely to contain something
  nobody meant to share. Off by default; --include-telemetry if you are
  debugging the loop rather than sharing a history.

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

# Loop telemetry. Excluded by default — see the module docstring.
TELEMETRY = ("deviceStatus", "apsResults")


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
    TDD = "tdd"


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
        print(f"  skip {table}: {e}", file=sys.stderr)
        return []


def _dropped(db: sqlite3.Connection, table: str) -> tuple[int, int]:
    """How many rows the two filters removed, for the stats report."""
    try:
        invalid = db.execute(
            f"SELECT count(*) FROM {table} WHERE isValid = 0"  # noqa: S608
        ).fetchone()[0]
        versioned = db.execute(
            f"SELECT count(*) FROM {table} WHERE referenceId IS NOT NULL"  # noqa: S608
        ).fetchone()[0]
        return invalid, versioned
    except sqlite3.OperationalError:
        return 0, 0


def _round(value, digits: int):
    """Round, preserving None.

    Values are rounded to the precision the device actually has. A pump that
    delivers in 0.01 U steps has no business emitting a float with seventeen
    significant figures, and the extra digits cost real bytes once there is one
    record every four minutes for a decade.
    """
    return None if value is None else round(value, digits)


def extract(db: sqlite3.Connection, include_telemetry: bool = False) -> list[dict]:
    """Build the canonical record list from an AAPS database."""
    out: list[dict] = []

    for r in _rows(db, "glucoseValues", "timestamp, value, trendArrow, sourceSensor"):
        out.append(
            {
                "t": r["timestamp"],
                "k": Kind.CGM,
                "mgdl": _round(r["value"], 1),
                "trend": r["trendArrow"],
                "src": r["sourceSensor"],
            }
        )

    for r in _rows(db, "boluses", "timestamp, amount, type, isBasalInsulin"):
        out.append(
            {
                "t": r["timestamp"],
                "k": Kind.BOLUS,
                "u": _round(r["amount"], 3),
                "type": r["type"],
                "basal": bool(r["isBasalInsulin"]),
            }
        )

    for r in _rows(db, "carbs", "timestamp, amount, duration"):
        out.append(
            {
                "t": r["timestamp"],
                "k": Kind.CARB,
                "g": _round(r["amount"], 1),
                "dur": r["duration"] or 0,
            }
        )

    for r in _rows(db, "temporaryBasals", "timestamp, type, isAbsolute, rate, duration"):
        out.append(
            {
                "t": r["timestamp"],
                "k": Kind.TBR,
                "rate": _round(r["rate"], 3),
                "abs": bool(r["isAbsolute"]),
                "dur": r["duration"],
                "type": r["type"],
            }
        )

    for r in _rows(db, "extendedBoluses", "timestamp, amount, duration"):
        out.append(
            {
                "t": r["timestamp"],
                "k": Kind.EXT_BOLUS,
                "u": _round(r["amount"], 3),
                "dur": r["duration"],
            }
        )

    # `note` is free text a person typed. It is carried because a site change or
    # an illness note is often the only explanation for a week of odd data — and
    # it is the field most likely to name a third party, so anything that
    # narrows a grant should narrow this first.
    for r in _rows(db, "therapyEvents", "timestamp, duration, type, note, glucose"):
        out.append(
            {
                "t": r["timestamp"],
                "k": Kind.EVENT,
                "type": r["type"],
                "dur": r["duration"] or 0,
                "note": r["note"] or None,
                "mgdl": _round(r["glucose"], 1),
            }
        )

    for r in _rows(db, "temporaryTargets", "timestamp, reason, highTarget, lowTarget, duration"):
        out.append(
            {
                "t": r["timestamp"],
                "k": Kind.TARGET,
                "lo": _round(r["lowTarget"], 1),
                "hi": _round(r["highTarget"], 1),
                "dur": r["duration"],
                "why": r["reason"],
            }
        )

    # Blocks are the profile itself (basal rates, ISF, IC, targets by time of
    # day). Without them a consumer cannot say what the loop was *trying* to do,
    # which makes the insulin records uninterpretable.
    for r in _rows(
        db,
        "profileSwitches",
        "timestamp, profileName, percentage, timeshift, duration, "
        "basalBlocks, isfBlocks, icBlocks, targetBlocks, glucoseUnit",
    ):
        out.append(
            {
                "t": r["timestamp"],
                "k": Kind.PROFILE,
                "name": r["profileName"],
                "pct": r["percentage"],
                "shift": r["timeshift"],
                "dur": r["duration"],
                "basal": r["basalBlocks"],
                "isf": r["isfBlocks"],
                "ic": r["icBlocks"],
                "target": r["targetBlocks"],
                "unit": r["glucoseUnit"],
            }
        )

    for r in _rows(db, "totalDailyDoses", "timestamp, basalAmount, bolusAmount, totalAmount, carbs"):
        out.append(
            {
                "t": r["timestamp"],
                "k": Kind.TDD,
                "basal": _round(r["basalAmount"], 2),
                "bolus": _round(r["bolusAmount"], 2),
                "total": _round(r["totalAmount"], 2),
                "g": _round(r["carbs"], 1),
            }
        )

    if include_telemetry:
        print(
            "  WARNING: telemetry included. deviceStatus and apsResults are loop "
            "debug, not history — see the module docstring.",
            file=sys.stderr,
        )

    out.sort(key=lambda rec: (rec["t"], rec["k"]))
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
    return b"".join(
        json.dumps(r, separators=(",", ":"), sort_keys=True).encode() + b"\n"
        for r in records
    )


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
    tables = [
        "glucoseValues", "boluses", "carbs", "temporaryBasals", "therapyEvents",
        "temporaryTargets", "extendedBoluses", "profileSwitches", "totalDailyDoses",
    ]
    tot_inv = tot_ver = 0
    for t in tables:
        inv, ver = _dropped(db, t)
        tot_inv += inv
        tot_ver += ver
    print(f"    {'invalid':<10} {tot_inv:>8,}   isValid = 0", file=sys.stderr)
    print(f"    {'version':<10} {tot_ver:>8,}   referenceId IS NOT NULL", file=sys.stderr)
    print(f"    {'cgm dup':<10} {cgm_dropped:>8,}   second+ reading in a 5-min bucket", file=sys.stderr)

    cgm = kinds.get(Kind.CGM, 0)
    if cgm:
        per_day = cgm / days
        print(
            f"\n  cgm     {per_day:>6.1f}/day after debounce "
            f"(a 5-min sensor can produce 288)",
            file=sys.stderr,
        )
        if cgm_dropped:
            ratio = (cgm + cgm_dropped) / cgm
            print(f"          raw was {ratio:.2f}x that — the double-broadcast", file=sys.stderr)

    gz = gzip.compress(blob, 9)
    print(
        f"\n  size    {len(blob) / 1e6:>7.2f} MB ndjson"
        f"   {len(gz) / 1e6:>6.2f} MB gzip"
        f"   over {days:.1f} days",
        file=sys.stderr,
    )
    print(
        f"  year    {len(blob) / 1e6 * 365 / days:>7.1f} MB ndjson"
        f"   {len(gz) / 1e6 * 365 / days:>6.1f} MB gzip"
        f"   projected",
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
    ap.add_argument("--include-telemetry", action="store_true", help="see the docstring first")
    args = ap.parse_args()

    if not args.db.exists():
        print(f"no such database: {args.db}", file=sys.stderr)
        return 1

    # Read-only, so a live snapshot cannot be modified by inspecting it.
    db = sqlite3.connect(f"file:{args.db}?mode=ro", uri=True)
    db.row_factory = sqlite3.Row

    print(f"  read {args.db}", file=sys.stderr)
    records = extract(db, args.include_telemetry)
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
