# The stall fix, verified — 2026-09-16 20:06

Same experiment that exposed the bug, with the fixed binary. Two desktop peers,
identical but for `--pass-secs`.

## Memory: 44× lower at the minute that mattered

| minute | broken | fixed |
|---|---|---|
| t+3 | 1,914 MB | 599 MB |
| t+4 | 2,052 MB | 852 MB |
| t+5 | 2,888 MB | 358 MB |
| **t+6** | **5,388 MB** | **122 MB** |
| t+7 | — | 739 MB ← one re-subscribe |
| t+8 | — | **46 MB** |

**The shape is the result, not the ratio.** The broken build climbed
monotonically to 5.4 GB. The fixed build sawtooths and *comes back down* — 122,
then 739 on a single re-subscribe, then 46. Transient catch-up allocation
returns; a leak does not.

⚠️ **The two runs carried different subjects**, so this is indicative rather
than a controlled A/B — the currency question needed a live publisher and the
broken run had none. What *is* controlled is the mechanism: `re-subs=0` through
t+6, where the broken build had been re-subscribing every 45 seconds.

## Currency: no regression, and the remedy still works

This was the risk — a fix that buys memory by starving the data path.

```
STALLED 12 passes — nothing new is arriving
stalled 180s — re-subscribed 5 topic(s) (1/5)
holding 2330 → 2332        ← STALLED gone
```

**The stall was real** — `552f688a` was demonstrably publishing throughout
(Ayni's unreadable count climbed 162 → 168 in the same window) while this peer
received nothing. The remedy fired at exactly 180 s, data resumed, the flag
cleared.

**So the fix delays the remedy; it does not suppress it.** Ingestion through the
run: +42/min, +1,453/min during catch-up, then +2/min at steady state.

## What one re-subscribe costs

**122 MB → 739 MB, for one.** That is the entire mechanism in a single data
point: each re-subscribe re-triggers catch-up over everything held and leaves a
p2panda broadcast ring behind. The broken build fired one every 45 seconds, so
they compounded before the previous could be released. That is the 5.4 GB.

🔑 **It also says the backoff matters more than the threshold.** At ~600 MB a go,
what protects a peer is not firing a second and third time into a stall that
cannot be fixed.

**And it is the number that makes the upstream defect worth reporting.** If
p2panda-net reclaimed a torn-down subscription, the remedy would be nearly free
and none of this would need tuning.
