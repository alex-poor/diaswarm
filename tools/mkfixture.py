#!/usr/bin/env python3
"""
Write a synthetic AAPS-shaped database, so canon.py can be run and tested by
someone who has no diabetes data.

WHY THIS EXISTS. Real snapshots never enter this repo (.gitignore refuses
`*.db`), which is right — and it means every number in the README and the spec
is unreproducible by anyone else, and the filters the whole protocol claim rests
on have nothing exercising them. This fixture is small, fake, and deliberately
contains one of each hazard:

  * version rows        `referenceId` set, in a bucket of their own so the
                        debounce cannot be mistaken for the filter that
                        removed them
  * retracted rows      `isValid = 0`
  * a retracted version row, which is both, to catch double-counting
  * two CGM readings in one 5-minute bucket, with DIFFERENT values so a test
    can tell "kept the first" from "kept the last"
  * a carb entry with a NULL duration, and one with a real zero, which must
    not survive as the same record
  * durations in milliseconds, the unit v1 of the spec got wrong
  * a utcOffset of +12h, the phase epochs are cut at
  * profiles in MMOL, in MGDL, and in a unit nobody recognises
  * an `extendedBoluses` table missing `referenceId`, i.e. an older schema,
    which must be reported as unread rather than silently yielding no records

Usage:
    tools/mkfixture.py out.db            # deterministic
    tools/mkfixture.py out.db --shuffle  # same rows, different insertion order
"""

from __future__ import annotations

import argparse
import random
import sqlite3
from pathlib import Path

# A fixed instant, so the fixture is byte-identical run to run. ALIGNED to a
# 5-minute bucket boundary on purpose: with an unaligned T0 the "second reading
# in the same bucket" below silently lands in the NEXT bucket, the debounce
# collides a different pair, and the test that says it keeps the first reading
# passes without ever exercising it.
T0 = 1_699_999_800_000
assert T0 % (5 * 60_000) == 0, "T0 must sit on a 5-minute bucket boundary"
MIN = 60_000

# AAPS stamps utcOffset on every row, and canon.py derives the epoch phase from
# it. A fixture without it would exercise a code path nobody runs.
TRACEABLE = ("id INTEGER PRIMARY KEY, isValid INTEGER NOT NULL, referenceId INTEGER, "
             "utcOffset INTEGER NOT NULL DEFAULT 43200000")

SCHEMA = {
    "glucoseValues": f"{TRACEABLE}, timestamp INTEGER, value REAL, trendArrow TEXT, sourceSensor TEXT",
    "boluses": f"{TRACEABLE}, timestamp INTEGER, amount REAL, type TEXT, isBasalInsulin INTEGER",
    "carbs": f"{TRACEABLE}, timestamp INTEGER, amount REAL, duration INTEGER",
    "temporaryBasals": f"{TRACEABLE}, timestamp INTEGER, type TEXT, isAbsolute INTEGER, rate REAL, duration INTEGER",
    "therapyEvents": f"{TRACEABLE}, timestamp INTEGER, duration INTEGER, type TEXT, note TEXT, glucose REAL",
    "temporaryTargets": f"{TRACEABLE}, timestamp INTEGER, reason TEXT, highTarget REAL, lowTarget REAL, duration INTEGER",
    "profileSwitches": (
        f"{TRACEABLE}, timestamp INTEGER, profileName TEXT, percentage INTEGER, timeshift INTEGER, "
        "duration INTEGER, basalBlocks TEXT, isfBlocks TEXT, icBlocks TEXT, targetBlocks TEXT, glucoseUnit TEXT"
    ),
    "totalDailyDoses": f"{TRACEABLE}, timestamp INTEGER, basalAmount REAL, bolusAmount REAL, totalAmount REAL, carbs REAL",
    # Deliberately an older schema: no referenceId. canon.py must report it as
    # unread, not quietly emit nothing.
    "extendedBoluses": "id INTEGER PRIMARY KEY, isValid INTEGER NOT NULL, timestamp INTEGER, amount REAL, duration INTEGER",
}

BASAL = '[{"duration":86400000,"amount":0.5}]'
ISF_MMOL = '[{"duration":86400000,"amount":2.0}]'
IC = '[{"duration":86400000,"amount":10.0}]'
TARGET_MMOL = '[{"duration":86400000,"lowTarget":5.0,"highTarget":6.0}]'
ISF_MGDL = '[{"duration":86400000,"amount":36.0}]'
TARGET_MGDL = '[{"duration":86400000,"lowTarget":90.0,"highTarget":108.0}]'


def rows() -> list[tuple[str, dict]]:
    """Every fixture row, as (table, column-values). Order here is not meaningful."""
    out: list[tuple[str, dict]] = []

    def add(table, **kw):
        kw.setdefault("isValid", 1)
        kw.setdefault("referenceId", None)
        out.append((table, kw))

    # --- CGM: five clean readings, five minutes apart ----------------------
    for i in range(5):
        add("glucoseValues", timestamp=T0 + i * 5 * MIN, value=100.0 + i,
            trendArrow="FLAT", sourceSensor="Dexcom G6")

    # A second reading inside bucket 0, two minutes later and 50 mg/dL apart.
    # Whichever one survives, a test can tell which.
    add("glucoseValues", timestamp=T0 + 2 * MIN, value=150.0,
        trendArrow="FLAT", sourceSensor="Dexcom G6")

    # Version history: a corrected copy of an earlier reading, written into its
    # own bucket with a value nothing else uses. In its own bucket deliberately —
    # a version row sharing a bucket with the row it supersedes would be removed
    # by the debounce too, and the test could not tell which filter did the work.
    add("glucoseValues", timestamp=T0 + 50 * MIN, value=777.0, trendArrow="FLAT",
        sourceSensor="Dexcom G6", referenceId=1)
    # Retracted.
    add("glucoseValues", timestamp=T0 + 40 * MIN, value=999.0, trendArrow="FLAT",
        sourceSensor="Dexcom G6", isValid=0)
    # Both retracted AND a version row — must be counted once, not twice.
    add("glucoseValues", timestamp=T0 + 45 * MIN, value=888.0, trendArrow="FLAT",
        sourceSensor="Dexcom G6", isValid=0, referenceId=2)

    # --- insulin and carbs -------------------------------------------------
    add("boluses", timestamp=T0 + 10 * MIN, amount=1.25, type="NORMAL", isBasalInsulin=0)
    add("boluses", timestamp=T0 + 11 * MIN, amount=0.05, type="SMB", isBasalInsulin=1)
    add("boluses", timestamp=T0 + 12 * MIN, amount=2.0, type="NORMAL",
        isBasalInsulin=0, referenceId=1)

    # A carb entry the device gave no duration for, and one with a real zero.
    # These are different facts and must stay different.
    add("carbs", timestamp=T0 + 10 * MIN, amount=30.0, duration=None)
    add("carbs", timestamp=T0 + 20 * MIN, amount=12.0, duration=0)
    add("carbs", timestamp=T0 + 25 * MIN, amount=99.0, duration=0, isValid=0)

    # Durations are MILLISECONDS, as AAPS stores them and as spec §2 says since
    # v2. Writing 30 here would mean thirty milliseconds, and a fixture that is
    # wrong about units teaches the wrong thing to whoever reads it first.
    add("temporaryBasals", timestamp=T0 + 5 * MIN, type="NORMAL", isAbsolute=1,
        rate=0.75, duration=30 * MIN)
    add("temporaryBasals", timestamp=T0 + 35 * MIN, type="EMULATED_PUMP_SUSPEND",
        isAbsolute=0, rate=0.0, duration=15 * MIN)

    add("therapyEvents", timestamp=T0 + 15 * MIN, duration=0, type="SENSOR_CHANGE",
        note=None, glucose=None)
    add("therapyEvents", timestamp=T0 + 16 * MIN, duration=0, type="NOTE",
        note="felt low", glucose=None)
    add("therapyEvents", timestamp=T0 + 17 * MIN, duration=0, type="FINGER_STICK",
        note=None, glucose=94.0)

    add("temporaryTargets", timestamp=T0 + 18 * MIN, reason="HYPOGLYCEMIA",
        highTarget=160.2, lowTarget=160.2, duration=45 * MIN)

    add("totalDailyDoses", timestamp=T0, basalAmount=12.0, bolusAmount=18.5,
        totalAmount=30.5, carbs=180.0)

    # --- profiles, one per unit case ---------------------------------------
    add("profileSwitches", timestamp=T0 + 1 * MIN, profileName="mmol", percentage=100,
        timeshift=0, duration=0, basalBlocks=BASAL, isfBlocks=ISF_MMOL, icBlocks=IC,
        targetBlocks=TARGET_MMOL, glucoseUnit="MMOL")
    add("profileSwitches", timestamp=T0 + 2 * MIN, profileName="mgdl", percentage=90,
        timeshift=0, duration=0, basalBlocks=BASAL, isfBlocks=ISF_MGDL, icBlocks=IC,
        targetBlocks=TARGET_MGDL, glucoseUnit="MGDL")
    add("profileSwitches", timestamp=T0 + 3 * MIN, profileName="odd", percentage=100,
        timeshift=0, duration=0, basalBlocks=BASAL, isfBlocks=ISF_MMOL, icBlocks=IC,
        targetBlocks=TARGET_MMOL, glucoseUnit="FURLONGS")

    # Old schema, no referenceId column — must surface as unread.
    out.append(("extendedBoluses", {"isValid": 1, "timestamp": T0 + 22 * MIN,
                                    "amount": 1.0, "duration": 30}))
    return out


def build(path: Path, shuffle_seed: int | None = None) -> None:
    if path.exists():
        path.unlink()
    db = sqlite3.connect(path)
    for table, cols in SCHEMA.items():
        db.execute(f"CREATE TABLE {table} ({cols})")  # noqa: S608 - constants above

    data = rows()
    if shuffle_seed is not None:
        random.Random(shuffle_seed).shuffle(data)

    for table, values in data:
        names = ", ".join(values)
        marks = ", ".join("?" * len(values))
        db.execute(
            f"INSERT INTO {table} ({names}) VALUES ({marks})",  # noqa: S608
            tuple(values.values()),
        )
    db.commit()
    db.close()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[1])
    ap.add_argument("out", type=Path, help="database to write")
    ap.add_argument("--shuffle", type=int, nargs="?", const=1, default=None,
                    help="insert the same rows in a different order (seed)")
    args = ap.parse_args()
    build(args.out, args.shuffle)
    print(f"  wrote {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
