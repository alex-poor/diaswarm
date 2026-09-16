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

## 🔴 CORRECTION, 19:45 — the first version of this got the trigger wrong

I wrote that a *quiet carried stranger* trips the stall detector, because
`worst = stale.maxOf { … }` takes the worst case across subjects. **That is not
what is happening.** `Follower.following()` filters to subjects whose keys we
hold — the ones we *follow* — so a stranger we merely carry is never in that
list at all.

**What is actually stale is one subject, and the number says everything:**

```
keys newest for 552f688a: 14908s old
```

552f688a is the loop phone. 14,908 seconds before 19:43 is **15:34** — the exact
minute [the unannounced rotation](../decisions.md) made everything sealed
afterwards unreadable to this follower.

## The chain, end to end

1. **The rotation blackout froze the newest *readable* record at 15:34.**
   Ciphertext keeps arriving — the `unreadable` counter climbs all afternoon —
   but nothing opens, so "age of newest record" stops advancing.
2. **The stall detector reads that as a transport stall**, because from where it
   stands the two look identical: no new data.
3. **So the remedy fires every fifteen minutes**, and has for four hours.
4. **`restream()` is a teardown and rebuild, not a refresh.** It aborts the task
   and drops the handle — dropping is what unsubscribes — then `stream()`s every
   topic again.
5. **Each re-subscribe makes p2panda-net allocate a fresh
   `broadcast::channel(1024)`** (~176 KB) and the old is not reclaimed. Exactly
   what heapprofd found: rings for topics *already subscribed*.
6. **It can never succeed.** The fault is cryptographic; re-subscribing a
   transport that is working fixes nothing. So it retries forever, and leaks
   forever.

⚠️ **AND PART OF TODAY'S LEAK IS AN ARTEFACT OF TODAY'S BUG.** The rotation
defect I shipped at 15:34 is what has been driving this phone's re-subscribes
since. **But the leak predates it** — the morning soak grew 138 → 263 MB from
08:00, long before any rotation — so the mechanism is general and some *other*
stall was driving it then. Do not let the tidy story hide that.

## Why the shape finally makes sense

Everything that did not fit now does:

* **Stepwise, not a ramp.** The growth is per stall-remedy, every fifteen
  minutes, not per second. A line fitted to it was always the wrong summary.
* **The rate varies tenfold between windows.** Stalls are bursty.
* **The fast peer at 8× the pass rate ran its stall check 8× as often** and grew
  catastrophically rather than 8× — because each re-subscribe also re-triggers
  catch-up over everything it holds. Its stall was real and ordinary: three
  passes with nothing new, which at 15 s is 45 seconds.
* **The slow peer recovered**, settling at ~150 MB after catch-up.

## The arithmetic, which is close but not exact

Phone B: 11 topics re-subscribed every 15 min = **0.129 MB/min** of rings, against
a measured floor of **+0.31 MB/min**.

⚠️ **The gap is a factor of ~2.4 and it is not explained.** Rings alone do not
account for it, so something else is probably retained per re-subscribe as well
— the `TopicManager` actor and its state are the obvious candidates. **Do not
write "the rings are the leak" as though it were settled; the mechanism is
established and the accounting is not.**

## The fix, which is sharper than "re-subscribe less"

**Ayni already knows the difference and is not using it.** As of this afternoon
`follow::read` reports `not_ours` separately from `lost()` — segments arriving
and refusing to open is *access control working*, and it means the transport is
fine.

> **If ciphertext is arriving and failing to open, that is not a transport
> stall, and re-subscribing is the wrong remedy.**

That single test would have stopped this afternoon's four hours of pointless
re-subscribes dead, and it is exactly the distinction the clinician work needed
for a different reason.

**Defence in depth, because a remedy that never works should not run forever:**
back off or cap. Four hours of fifteen-minute retries that cannot possibly
succeed is a design gap independent of why they could not succeed.

p2panda-net not reclaiming a torn-down subscription is still a real upstream
defect and still worth reporting — but we would stop hitting it.

## What this was measured with

`--pass-secs`, added to `diaswarm-peer` for exactly this experiment. ⚠️ **A low
value is dangerous**: 15 s took a peer to 5.4 GB in six minutes on a laptop with
30 GB. It is a diagnostic, not a tuning knob.
