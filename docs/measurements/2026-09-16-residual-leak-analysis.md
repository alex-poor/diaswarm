# The residual leak, re-measured — 2026-09-16 14:30

Re-analysis of `2026-09-16-ayni-memory.txt` plus the loop phone's parallel curve.
**Three of the handover's §1 statements need correcting.** No new run was needed;
every number below comes from data already collected.

---

## 1. The residual is +0.52 MB/min, not +0.7 — and the pool was the confound

`native_kb` tracks the sqlx connection count at **≈5 MB per connection**. The pool
swings between 11 and 20 over the run, so ±8 connections is ±40 MB — enough to
swamp half an hour of leak in either direction.

Regressing native on time *and* pool size:

| window | raw | pool-adjusted |
|---|---|---|
| whole run (t+0…371) | +0.468 | **+0.481** MB/min |
| chunk 1, warm-up included | +1.002 | +0.796 |
| chunk 2, steady (t+215…371) | +0.705 | **+0.524** |
| last 71 min | +0.319 | **+0.529** |

**Steady-state answer: +0.52 MB/min.** The handover's +0.7 came from endpoint
subtraction (138 → 263 over 3 h) across a stretch where the pool also grew.

### This is the mechanism behind "three samples can look flat"

Look at the last 71 minutes: raw **+0.32**, pool-adjusted **+0.53**. The pool shrank
20 → 12 in that window and gave back ~40 MB, hiding the climb. The handover's
warning was right; this is *why*. Always read the pool count next to the memory.

### The pool is not the leak

At the *same* pool size (16–18 connections), native is **133 MB in the first hour
and 265 MB after t+300** — 132 MB apart. Pool size explains the wobble, not the trend.

---

## 2. 🔴 The loop phone leaks at the same rate on UNPATCHED iroh

`curve-loop.log`, AAPS on the loop phone, 86 samples over 171 min, process uptime
16 h 40 m, **stock iroh 1.1.0 with no patch**:

```
native PSS  +0.573 MB/min    (136 MB mean in the first 30 min -> 199 MB in the last 30)
```

Against phone B's patched Ayni at **+0.52 MB/min**. Same slope, two different apps,
one patched and one not.

**Therefore the residual is not `pending_open_paths`.** It is in code both apps
share, and the vendored iroh patch neither causes nor cures it. Two consequences:

* `vendor/iroh` is exonerated — don't go looking there, and don't let the residual
  count against keeping the patch. The patch still fixes what it claimed to fix.
* **You do not need phone B to chase this.** An unpatched 16-hour-old process
  reproducing the same slope is plugged in right now.

---

## 3. 🔴 Every curve so far recorded the wrong number

`rss-curve.sh` and `loop-curve.sh` both do:

```sh
dumpsys meminfo $pid | grep -m1 "Native Heap:" | tr -s " " | cut -d" " -f4
```

That matches the **App Summary** line, not the MEMINFO table row, and field 4 there
is **Pss**. Loop phone, same instant:

```
pss=200490  swap=203590  rss=201580  size=519744  alloc=415256  free=88978
```

* **PSS says 200 MB. The allocator has 415 MB outstanding.**
* **204 MB is swapped into zram** — and PSS does not count swapped pages.

So a leak can *flatten a PSS curve purely by being compressed away*, and the real
footprint is roughly double what every chart in this repo shows. `Heap Alloc` is
the honest number for a leak: what the allocator handed out and has not got back.

This is the same family as the withdrawn "flat memory curve" claim. Measuring less
directly than the claim needed, again.

**Fixed:** `curve2.sh` records `pss/swap/rss/size/alloc/free`, running on the loop
phone as `LOOPV2` from 14:29. Historical curves are still usable for *shape*, but
their absolute values understate and their flat stretches are not trustworthy.

---

## 4. Why the soak stopped: adb, not the FGS

Last good sample **t+371m (14:12:05)**; blanks every minute since. Phone B is absent
from `adb devices`.

It was **not** the 6-hour foreground-service kill:

* `rss-curve.sh` prints `GONE` when `pidof` finds nothing, and a **blank** when adb
  itself fails. We got blanks.
* A 6 h kill would have landed at 14:00:51. Samples reported a live `pid=6562`
  through 14:12.

**So the process on phone B is probably still alive and now ~6.5 h old.** Replug the
USB and the soak resumes against the same pid — which is the heapprofd target the
handover asked for. If `pidof` comes back empty after replugging, *then* the FGS
kill happened somewhere in the blind window.

⚠️ The committed `2026-09-16-ayni-memory.txt` is a 14:13 snapshot. The live log is
`curve-patched.log` in the previous session's scratchpad.

---

## What this changes

1. Residual leak: **+0.52 MB/min**, present on patched and unpatched builds alike,
   so it is ours, not iroh's.
2. At that rate the six hours Android actually allows the service is **~190 MB of
   drift** — real, but not what kills the process. The FGS limit still does.
3. Profile `Heap Alloc`, not PSS, and always read the sqlx pool count beside it.
