# The leak is our own stall remedy — 2026-09-16 19:45

**Found by running two desktop peers that differed only in pass rate.** The fast
one blew up; its log said why in plain words.

```
FAST  --pass-secs 15    916 MB → 5,388 MB in six minutes
SLOW  --pass-secs 120   535 MB → peaked 899 MB → settled at ~150 MB
```

```
    stalled — re-subscribed 7 topic(s)
    ... STALLED 7 passes — nothing new is arriving
```

---

## The chain, end to end

1. **Ayni carries strangers' ciphertext.** That is not incidental, it is the
   product — the app is named for it.
2. **A carried stranger who is not publishing is perfectly normal.** They are
   quiet; there is nothing to receive.
3. **The stall test takes the worst case across every subject:**
   `val worst = stale.maxOf { it.second }` (`SyncWorker.kt`). One quiet stranger
   puts `worst` permanently over the threshold.
4. **So the remedy fires forever.** On phone B, right now: *"keys stalled for
   14702s — re-subscribed 11 topic(s)"*, every fifteen minutes, having been
   "stalled" for four hours.
5. **`restream()` is not a refresh — it is a teardown and a rebuild.** It aborts
   the task, drops the handle (which is what unsubscribes), then calls
   `stream()` again for every topic.
6. **Each re-subscribe makes p2panda-net allocate a fresh
   `broadcast::channel(1024)` ring**, ~176 KB, and the previous one is not
   reclaimed. That is exactly what heapprofd found: rings created for topics
   *already subscribed*.

**So the memory does not leak while it works. It leaks while it is trying to fix
itself** — and it is always trying, because carrying a quiet stranger is
indistinguishable from being broken.

## Why the shape finally makes sense

Everything that did not fit now does:

* **Stepwise, not a ramp.** The growth is per stall-remedy, every fifteen
  minutes, not per second. A line fitted to it was always the wrong summary.
* **The rate varies tenfold between windows.** Stalls are bursty.
* **The fast peer at 8× the pass rate ran its stall check 8× as often** and grew
  catastrophically rather than 8× — because each re-subscribe also re-triggers
  catch-up over everything it holds.
* **The slow peer recovered**, settling at ~150 MB after catch-up.

## The arithmetic, which is close but not exact

Phone B: 11 topics re-subscribed every 15 min = **0.129 MB/min** of rings, against
a measured floor of **+0.31 MB/min**.

⚠️ **The gap is a factor of ~2.4 and it is not explained.** Rings alone do not
account for it, so something else is probably retained per re-subscribe as well
— the `TopicManager` actor and its state are the obvious candidates. **Do not
write "the rings are the leak" as though it were settled; the mechanism is
established and the accounting is not.**

## The fix is ours, not upstream

p2panda-net not reclaiming a torn-down subscription is a real upstream defect and
worth reporting. But **we should not be tearing them down every fifteen minutes
in the first place.**

`worst = max(staleness)` is the bug. A subject this phone merely *carries* going
quiet is not a stall — it is Tuesday. The test should be per-topic, and should
only fire for a topic we have reason to expect data from.

**That single change would stop the leak at its source** without waiting on
anyone, and would also stop a pointless full re-subscribe of eleven topics every
quarter of an hour.

⚠️ **AND THE REMEDY IS NOT REMEDYING.** Four hours "stalled", re-subscribing
every fifteen minutes, still stalled. Whatever it was written to fix, it is not
fixing that either — so this is currently pure cost. Worth establishing what it
was supposed to do before deciding whether to keep it at all.

## What this was measured with

`--pass-secs`, added to `diaswarm-peer` for exactly this experiment. ⚠️ **A low
value is dangerous**: 15 s took a peer to 5.4 GB in six minutes on a laptop with
30 GB. It is a diagnostic, not a tuning knob.
