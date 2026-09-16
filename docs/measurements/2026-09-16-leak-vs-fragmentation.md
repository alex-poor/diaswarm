# The leak is three things, and PSS was adding them up — 2026-09-16 16:45

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

## 3. And there is still a real live leak, about half what was claimed

**+0.235 MB/min of live, outstanding allocation with the pool pinned at its
ceiling.** Nothing about connections explains it. This is where the p2panda
broadcast rings from
`2026-09-16-residual-leak-profiled.md` live.

🔴 **THE +0.52 MB/min FIGURE IS WITHDRAWN.** It came from native-heap **PSS**,
which sums live allocation, in-heap free space, and pool churn into one number
and cannot tell them apart. The honest live-leak rate on this run is
**+0.24 MB/min**, under half of it. The arena figure, +0.51, is the one that
matches the old PSS curves — which is the coincidence that made the wrong number
look right.

| claim | status |
|---|---|
| +0.7 MB/min residual | withdrawn 2026-09-16 (pool confound) |
| +0.52 MB/min residual | **withdrawn now** (PSS conflates three things) |
| +0.24 MB/min live, +0.51 arena | current, 78 min, pool-controlled |

⚠️ **RATES ARE NOT PORTABLE BETWEEN RUNS.** The heapprofd capture measured
+2.0 MB/min on a 6.5-hour-old process under `block_client`. This is +0.24 on an
80-minute process with no profiler attached. Both are real; neither is "the
rate". Quote the conditions or do not quote the number.

---

## What this changes

1. **Stop looking at SQLite.** The cap holds, measured per connection.
2. **The live leak is the rings**, and it is ~0.24 MB/min, not 0.52.
3. **Half the footprint problem is not a leak at all** and would not be fixed by
   fixing the rings — it needs the allocator question answered first.
4. **Always log the pool count.** Three of this project's withdrawn memory
   claims came from a number that silently contained it.
