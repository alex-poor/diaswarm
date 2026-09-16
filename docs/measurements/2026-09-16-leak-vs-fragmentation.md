# The leak is three things, and PSS was adding them up — 2026-09-16 16:45

> ⚠️ **Revised 16:55**: §3 originally reported a live leak of +0.235 MB/min.
> That slope sits inside its own error bars and was withdrawn.
>
> 🔴 **REVISED AGAIN 18:30, AND THIS TIME THE LEAK IS REAL.** At 188 minutes the
> live trend is unmistakable and the "mostly free space" reading was an artifact
> of a window dominated by the quietest hour of the run. **Read §4 first; §1–3
> are kept for the record and §3's conclusion is wrong.**

78 minutes on phone B (Ayni 0.1.7, fresh process, past warm-up), sampling
`Heap Alloc`, `Heap Size`, `Heap Free` **and the sqlx connection count**
together for the first time. That last column is what makes this readable.

```
                       trend          per sqlx connection
  alloc (live)      +0.192 MB/min          +1.70 MB
  size  (arena)     +0.487 MB/min          -1.12 MB
  free  (in-heap)   +0.295 MB/min          -2.81 MB
```

**Restricted to samples where the pool is at its ceiling of 20** — 25 samples
spanning the whole 78 minutes, so the pool cannot explain anything:

```
  alloc +0.235   size +0.514   free +0.281 MB/min
```

`alloc + free = size` holds to within 0.002, so these are three views of one
arena and not three measurements that happen to disagree.

---

## 1. The SQLite cap works. It was never the leak.

`corr(free, sqlx) = -0.82`. When the pool shrinks, in-heap free space grows —
connections closing and handing their page caches back to the *allocator*, not
to the OS.

And the per-connection numbers say the cap is doing exactly what it was set to
do: **+1.70 MB of live allocation per connection**, against a `cache_size` of
`-2000` — a 2 MB ceiling. `cd8258a` was correct, [[bounded_store_pragmas]]
proves it reaches every pooled connection, and this is that pragma working,
measured from the outside on a real device.

**So the answer to "ceiling or second leak?" is ceiling.** The page cache is
bounded per connection and the pool is bounded at 20. It cannot grow without
limit and it does not.

## 2. But its churn ratchets the arena, and that never comes back

More than half of all growth — **+0.28 of the +0.51 MB/min** — is free space
accumulating *inside* the heap. The process asks the OS for more arena, then
frees into it, and the freed pages stay with the process.

That is why a pool that is bounded still costs: every cycle of connections
closing and reopening leaves the arena a little larger, permanently. Bounded
live usage, unbounded footprint.

⚠️ **Not yet established that this is fragmentation rather than Scudo simply
being lazy about returning pages.** Android's allocator decommits on its own
schedule. The measurement that separates them is `malloc_info` or a forced
`mallopt(M_PURGE)` — neither has been run. Do not call it fragmentation in a
commit message until one of them has.

## 3. 🔴 A LIVE LEAK IS NOT PROVEN BY THIS RUN — AND I NEARLY CLAIMED ONE

The first version of this note reported "+0.235 MB/min of live allocation" as a
finding. **It does not survive its own error bars**, and this project has
withdrawn five claims for exactly that.

Every trend above, tested against the scatter it sits in:

| | slope | SE | t | R² | resid sd | verdict |
|---|---|---|---|---|---|---|
| `free` (in-heap) | +0.281 | 0.049 | **5.7** | 0.58 | 5.9 MB | solid |
| `size` (arena) | +0.514 | 0.147 | **3.5** | 0.35 | 17.5 MB | solid |
| `alloc` (live) | +0.235 | 0.150 | **1.6** | 0.10 | 17.9 MB | **not separated from noise** |

`alloc` scatters by ±18 MB between consecutive samples — and the entire
78-minute trend is +18 MB. The trend is the same size as the noise. Watched
directly: `alloc` read 119.6 MB at 16:37 and 103.9 MB ten minutes later, a
16 MB *fall*.

**So the honest statement is: live allocation may be growing at ~0.24 MB/min, or
may be flat. 78 minutes cannot tell.** Separating it needs roughly 2.5× the
spread in time — about **4–5 hours** at this cadence. The sampler is still
running.

The per-connection coefficients in §1 *do* survive (`alloc` +1.70 MB, t=3.0;
`free` −2.81 MB, t=−12.8), which is why the cap conclusion stands while the
trend conclusion does not. `size` per connection (−1.12, t=−2.1) does not
survive either and is not claimed.

### What is withdrawn

🔴 **+0.52 MB/min as a "residual leak" is withdrawn.** It came from native-heap
**PSS**, which sums live allocation, in-heap free space and pool churn into one
number. The part of it that is real and measurable is **arena growth, +0.51
MB/min** — which is why the old PSS curves matched it so well, and exactly why
the wrong number looked right for so long.

| claim | status |
|---|---|
| +0.7 MB/min residual | withdrawn (pool confound) |
| +0.52 MB/min residual leak | **withdrawn** — it was arena growth, not live growth |
| +0.51 MB/min arena growth | holds, t=3.5 |
| +0.28 MB/min in-heap free growth | holds, t=5.7 |
| any live leak | **unproven at 78 min**; needs 4–5 h |

⚠️ **RATES ARE NOT PORTABLE BETWEEN RUNS.** The heapprofd capture measured
+2.0 MB/min on a 6.5-hour-old process under `block_client`. This is +0.24 on an
80-minute process with no profiler attached. Both are real; neither is "the
rate". Quote the conditions or do not quote the number.

---

## What this changes

1. **Stop looking at SQLite.** The cap holds, measured per connection.
2. **The growth that is actually proven is the arena, not live objects.** Fixing
   the p2panda rings might change nothing about the footprint curve — they are
   live allocations, and live allocation growth is not yet demonstrated.
3. **Do not quote a live-leak rate until the long run is in.** 4–5 hours.
4. **Always log the pool count**, and **always publish the error bars.** Three of
   this project's withdrawn claims came from a number that silently contained the
   pool; this one nearly came from a slope inside its own noise.


---

## 4. 🔴 At three hours: the leak is LIVE, and §3 was measured over the quiet hour

95 samples, 188 minutes, same process. With the pool pinned at its ceiling of 20
(75 of the 95 samples):

```
alloc (live)     +0.436 MB/min   SE 0.059   t = 7.4    SIGNIFICANT
size  (arena)    +0.565 MB/min   SE 0.054   t = 10.5   SIGNIFICANT
free  (in-heap)  +0.117 MB/min   SE 0.015   t = 7.8    small
```

**Live allocation went 89.1 → 173.1 MB. It nearly doubled in three hours, and
in-heap free space did not grow with it** — 36 MB at the start, 43 MB at the
end. So the growth is objects that are still reachable, not an arena filling
with holes.

### Why the 78-minute answer was wrong

The rate is not constant. In thirds:

| window | slope | t |
|---|---|---|
| t+0…60 | +0.13 MB/min | 0.7 |
| t+62…122 | **+1.32** MB/min | 6.8 |
| t+124…188 | **+0.95** MB/min | 2.9 |

**§3 analysed a window that was mostly the first third** — the one hour in the
run where almost nothing accumulated. That is why the slope looked like noise
and why free space looked dominant: it was, *in that hour*. Extending the run
did not refine the estimate, it reversed the conclusion.

⚠️ **SO DO NOT QUOTE A SINGLE MB/min FOR THIS.** The rate varies by a factor of
ten depending on the window. What is defensible is the total and the shape:
**live allocation roughly doubled over three hours, free space did not.** A
slope is a summary of a curve that is not a line.

### What this restores and what stays dead

| claim | status |
|---|---|
| +0.7 MB/min residual | still withdrawn — pool confound, and a PSS number |
| +0.52 MB/min | **closer to right than the correction that replaced it.** It was PSS, so it still conflated three things, but its magnitude was not the error |
| "live growth is unproven" (§3) | 🔴 **withdrawn** — it was 78 minutes over the quiet hour |
| "more than half the growth is in-heap free space" (§2) | 🔴 **withdrawn** — free space is ~20% of it over three hours |
| the SQLite cap works, +1.70 MB/connection | **holds** — §1 is unaffected |

**The p2panda broadcast rings are back to being the prime suspect**, and for the
right reason this time: the growth is live, retained allocation, which is what
heapprofd attributed to them.

### The lesson, which this project keeps paying for

Three hours reversed what 78 minutes concluded, and 78 minutes had already
reversed what 15 minutes concluded. **Every time the window has been extended
here, the answer has changed** — the page cache, the burst, the residual, and
now this. The handover's rule was "read hours, not samples"; the sharper version
is **read several hours, and check the thirds before believing the slope.**


---

## 5. At four hours: real, sustained, and **stepwise** — so a slope is the wrong summary

118 samples, 234 minutes:

```
alloc (live)  +0.485 MB/min   SE 0.037   t = 13.0     89 -> 182 MB
free          +0.095 MB/min              t =  6.1     19 ->  52 MB
```

The leak is not in doubt any more. **But the quarters are the finding:**

| window | slope | t | mean alloc |
|---|---|---|---|
| t+0…56 | +0.18 | 0.8 | 103 MB |
| t+58…114 | **+0.91** | 5.7 | 121 MB |
| t+116…172 | **−0.75** | −3.0 | 146 MB |
| t+174…234 | −0.45 | −1.4 | 196 MB |

🔴 **TWO QUARTERS HAVE NEGATIVE SLOPES WHILE THE LEVEL NEARLY DOUBLES.** That is
not noise — Q3 is t = −3.0. The process **steps up between windows and decays
within them**: sawtooth, not a ramp.

**So growth is driven by discrete events, not continuous accumulation.** That is
exactly the shape a per-pass allocation makes — and the suspect from the
heapprofd profile is a `broadcast::channel(1024)` ring re-created per sync pass.
A continuous leak would not decay inside a window.

⚠️ **AND IT MEANS EVERY SLOPE IN THIS DOCUMENT, INCLUDING THE GOOD ONES, IS A
SUMMARY OF SOMETHING THAT IS NOT A LINE.** Fitting a line to a staircase gives a
number that is true on average and describes no moment. Quote **89 → 182 MB over
four hours**, and the staircase; not a rate.

At the overall fit the process reaches ~264 MB at 6 h — comfortably inside what
Android allows before the foreground service is killed anyway, which is why this
is a defect and not an outage.
