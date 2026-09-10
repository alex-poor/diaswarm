#!/usr/bin/env python3
"""
Tests for canon.py, against the synthetic fixture. No dependencies, no network,
no real data:

    tools/test_canon.py

WHAT THESE ARE FOR. Everything downstream encodes against the record shape in
spec/records.md, and the filters in §3 are the difference between a consumer
seeing a person's history and seeing roughly twice their insulin. Those are the
claims under test here — not the plumbing.
"""

from __future__ import annotations

import json
import sqlite3
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

import canon  # noqa: E402
import mkfixture  # noqa: E402

FAILURES: list[str] = []


def check(name: str, condition: bool, detail: str = "") -> None:
    if condition:
        print(f"  ok    {name}")
    else:
        print(f"  FAIL  {name}   {detail}")
        FAILURES.append(name)


def stream(path: Path) -> list[dict]:
    canon.SKIPPED.clear()
    canon.UNKNOWN_UNITS.clear()
    db = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    db.row_factory = sqlite3.Row
    records, _ = canon.debounce_cgm(canon.extract(db))
    db.close()
    return records


def of_kind(records: list[dict], kind: str) -> list[dict]:
    return [r for r in records if r["k"] == kind]


def main() -> int:
    tmp = Path(tempfile.mkdtemp())
    fixture = tmp / "fixture.db"
    mkfixture.build(fixture)
    recs = stream(fixture)

    # --- §3.1 / §3.2: the two filters --------------------------------------
    cgm = of_kind(recs, "cgm")
    check("version rows are not events", not any(r["mgdl"] == 777.0 for r in cgm),
          "a referenceId-bearing row reached the stream")
    check("retracted rows are dropped",
          not any(r["mgdl"] in (999.0, 888.0) for r in cgm), f"{cgm}")
    check("retracted bolus version row dropped", len(of_kind(recs, "bolus")) == 2,
          f"got {len(of_kind(recs, 'bolus'))}")
    check("retracted carb dropped", len(of_kind(recs, "carb")) == 2,
          f"got {len(of_kind(recs, 'carb'))}")

    # --- §3.3: one CGM reading per bucket, KEEPING THE FIRST ----------------
    #
    # Eight, not five: five five-minute readings plus three buckets' worth of
    # the one-minute sensor the fixture now carries. That sensor is there
    # because §3.3 was implemented in this tool and NOT in the emitter the
    # phone runs, and with no sub-five-minute CGM in the fixture there was
    # nothing for the two to disagree about.
    check("one cgm reading per 5-minute bucket", len(cgm) == 8, f"got {len(cgm)}")

    # A one-minute sensor reaches a reader as a five-minute one, keeping the
    # first of each bucket — 200.0, 205.0, 210.0 rather than any later value.
    minute_sensor = sorted(r["mgdl"] for r in cgm if r["mgdl"] >= 200.0)
    check("a one-minute sensor is thinned to one reading per bucket",
          minute_sensor == [200.0, 205.0, 210.0], f"got {minute_sensor}")
    first_bucket = [r for r in cgm if r["t"] // canon.CGM_BUCKET_MS
                    == mkfixture.T0 // canon.CGM_BUCKET_MS]
    check("debounce keeps the FIRST reading of a bucket, not the last",
          len(first_bucket) == 1 and first_bucket[0]["mgdl"] == 100.0,
          f"got {first_bucket}")

    # --- §1: absent is not zero --------------------------------------------
    carbs = {r["g"]: r for r in of_kind(recs, "carb")}
    check("an unreported duration is absent, not zero", "dur" not in carbs[30.0],
          f"got {carbs[30.0]}")
    check("a real zero duration survives as zero", carbs[12.0].get("dur") == 0,
          f"got {carbs[12.0]}")
    check("no record carries an explicit null",
          all(v is not None for r in recs for v in r.values()))

    # --- §2: durations are milliseconds, the thing v1 got wrong -------------
    tbrs = of_kind(recs, "tbr")
    check("a temp basal duration is milliseconds, not minutes",
          any(r.get("dur") == 30 * 60_000 for r in tbrs),
          f"got {[r.get('dur') for r in tbrs]}")
    check("a temporary target duration is milliseconds",
          of_kind(recs, "target")[0].get("dur") == 45 * 60_000)
    check("the stream declares spec v3", canon.SPEC_VERSION == 3)

    # --- §2: units are fixed, never per-record ------------------------------
    profiles = {r["name"]: r for r in of_kind(recs, "profile")}
    check("profile blocks are parsed, not JSON inside JSON",
          isinstance(profiles["mmol"]["target"], list), f"{profiles['mmol']['target']!r}")
    check("mmol targets are normalised to mg/dL",
          profiles["mmol"]["target"][0]["lowTarget"] == round(5.0 * canon.MMOLL_TO_MGDL, 1),
          f"got {profiles['mmol']['target'][0]}")
    check("mmol ISF is normalised to mg/dL",
          profiles["mmol"]["isf"][0]["amount"] == round(2.0 * canon.MMOLL_TO_MGDL, 1),
          f"got {profiles['mmol']['isf'][0]}")
    check("mg/dL profiles are left alone",
          profiles["mgdl"]["target"][0]["lowTarget"] == 90.0,
          f"got {profiles['mgdl']['target'][0]}")
    check("unit-independent blocks are never scaled",
          profiles["mmol"]["basal"][0]["amount"] == 0.5
          and profiles["mmol"]["ic"][0]["amount"] == 10.0)
    check("normalised profiles carry no unit", "unit" not in profiles["mgdl"]
          and "unit" not in profiles["mmol"])
    check("an unrecognised unit is admitted, not guessed",
          profiles["odd"].get("unit") == "FURLONGS" and canon.UNKNOWN_UNITS,
          f"got {profiles['odd'].get('unit')!r}")
    check("a temporary target stays in mg/dL",
          of_kind(recs, "target")[0]["lo"] == 160.2)

    # --- §1: ordering two implementations can agree on ----------------------
    shuffled = tmp / "shuffled.db"
    mkfixture.build(shuffled, shuffle_seed=7)
    check("output does not depend on database row order",
          canon.encode(stream(shuffled)) == canon.encode(recs))
    check("records are ordered by (t, k)",
          all((a["t"], a["k"]) <= (b["t"], b["k"]) for a, b in zip(recs, recs[1:])))

    # --- a table that could not be read is not a table with no events -------
    stream(fixture)
    check("an unreadable table is reported, not silently empty",
          any(t == "extendedBoluses" for t, _ in canon.SKIPPED), f"{canon.SKIPPED}")

    # --- the dropped-row breakdown must not double-count --------------------
    db = sqlite3.connect(f"file:{fixture}?mode=ro", uri=True)
    invalid, versioned = canon._dropped(db, "glucoseValues")
    total = db.execute(
        "SELECT count(*) FROM glucoseValues WHERE isValid = 0 OR referenceId IS NOT NULL"
    ).fetchone()[0]
    db.close()
    check("dropped counts are disjoint", invalid + versioned == total,
          f"{invalid} + {versioned} != {total}")

    # --- v1: the stream says what it is ------------------------------------
    blob = canon.encode(recs)
    first = json.loads(blob.split(b"\n")[0])
    check("the stream declares its spec version",
          first.get("spec") == canon.SPEC_VERSION, f"{first}")
    check("the stream declares how epochs are cut", first.get("epoch") == "offset-day")
    check("and the phase they are cut at", "offset" in first, f"{first}")
    check("the stream declares its glucose unit", first.get("unit") == "mgdl")
    check("the header sorts ahead of every event", first["t"] == 0
          and all(r["t"] > 0 for r in recs))
    check("encode emits one line per record plus the header",
          blob.count(b"\n") == len(recs) + 1 and blob.endswith(b"\n"))

    # --- v1: tdd is not in the vocabulary ----------------------------------
    check("tdd is gone from the vocabulary", not of_kind(recs, "tdd")
          and not hasattr(canon.Kind, "TDD"))

    # --- v1: epochs are UTC days -------------------------------------------
    day = 86_400_000
    check("an epoch is a day wide",
          canon.epoch_of(0) == 0 and canon.epoch_of(day - 1) == 0
          and canon.epoch_of(day) == 1)
    check("the offset shifts the phase, not the width",
          canon.epoch_of(0, 12 * 3600_000) == 0
          and canon.epoch_of(12 * 3600_000, 12 * 3600_000) == 1,
          "a +12h offset must move the boundary to local midnight")
    check("epoch identity does not depend on where a record was written",
          canon.EPOCH_MS == day and canon.EPOCH_BASIS == "offset-day")

    print()
    if FAILURES:
        print(f"  {len(FAILURES)} FAILED: {', '.join(FAILURES)}")
        return 1
    print(f"  all {len(recs)} records checked, no failures")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
