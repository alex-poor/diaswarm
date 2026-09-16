<!-- DRAFT, NOT POSTED. Prepared 2026-09-16 for iroh#4390.
     Outward-facing under the repository owner's name, so it waits for them.
     Updated 2026-09-16 14:40: the 13-minute patch claim is replaced by the
     6-hour soak, and the residual drift is now attributed away from iroh
     (an unpatched build leaks at the same rate) -- so nothing here asks
     maintainers to account for a leak that is ours. -->

Third platform, with a symbolised Android stack and a measurement of the
doubling itself — in case it helps this off the backlog.

**Environment:** iroh 1.1.0 via p2panda-net 0.7.1, Android 15 (Pixel 7,
aarch64), a background app holding gossip + log-sync connections to a handful of
peers. Same code in 1.2.0, so we see it there too.

**Captured with heapprofd** on a process that had been up 54 minutes, symbolised
against an unstripped build:

```
realloc
alloc::raw_vec::RawVecInner::finish_grow
alloc::raw_vec::RawVecInner::grow_amortized
alloc::raw_vec::RawVec<iroh::socket::transports::FourTuple>::grow_one
alloc::collections::vec_deque::VecDeque<FourTuple>::push_back_mut
iroh::socket::remote_map::remote_state::State::open_path_on_conn
iroh::socket::remote_map::remote_state::RemoteStateActor::open_path_on_all_conns
```

Over a 150-second window, on that callsite alone:

| | |
|---|---|
| largest **single** allocation | 834,695,168 bytes (796 MB) |
| total allocated | 1.13 GB |
| **net retained** | **7.5 MB** |
| allocation records | 33 |

**The gap between those last two numbers is why this is easy to miss.** Because
`grow_amortized` doubles and frees the old buffer, anything that sums *net*
allocation attributes ~7 MB here and moves on — it looks unremarkable next to
ordinary churn. The 796 MB is only visible as `max(size)`. We spent a day
chasing SQLite page cache before querying it that way.

It also explains the shape users report. The doubling transiently holds both
buffers, so RSS jumps by hundreds of MB and then falls back, which reads as
"bursty" rather than "leaking" — and on Android, lowmemorykiller reaps the
process and it restarts, so it presents as a mysterious periodic restart rather
than an OOM. Ours looked like this:

```
t+6m .. t+15m   ~125-130 MB   flat        <- looks fine, and fooled us twice
t+43m            198 MB
t+48m            255 MB
t+54m            416 MB       (+127 MB inside one minute)
```

**Reproduction note that may be useful:** the trigger correlates with network
roams. @danscan's iOS report mentions WiFi ⇄ cellular roams, and we see the same
— our phones change network and the resulting path churn drives connections to
the path-id cap. Anyone trying to reproduce on a desktop may find it easier by
forcing interface changes than by waiting.

**We are running @cbenhagen's hot-fix from this thread** (dedup + cap at 64),
vendored, and it held for a **six-hour soak** — no bursts at all, where the
unpatched build on the same phone was at 348 MB by t+15m, 416 MB by t+54m and
then killed:

```
unpatched   348 MB at t+15m, 416 MB at t+54m (+127 MB in one minute), killed
patched     263 MB at t+6h,  no burst of any kind
```

To be clear about what that does *not* say: the patched build still drifts
**+0.5 MB/min**, but that drift is **not this bug**. We have a second phone
running an unpatched build that drifts at the same rate, so it is our own code,
and we mention it only so the number is not read against the fix.

For anyone else measuring: our `Native Heap` figures above come from `dumpsys
meminfo`'s App Summary, which reports **Pss** and therefore excludes pages
swapped into zram — on our device the allocator's outstanding total is roughly
double the PSS figure. The comparison is still apples to apples, but the
absolute numbers understate.

**So from our side the hot-fix is the fix**, and the open question is a
maintainer decision rather than a fourth patch: #4398 and #4414 were closed by
their own authors (one for want of "a clearer review path"), and #4522 is open
with conflicts and no review.

Happy to supply the raw perfetto trace or re-run with different instrumentation
if that's useful.
