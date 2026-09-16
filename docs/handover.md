# Handover — 2026-09-16 14:15

> ⚠️ **PARTLY SUPERSEDED, 15:05.** Read this block before acting on §1 or §3.
>
> * **§3.1 is DONE.** The residual leak is named: p2panda-net allocates a
>   `broadcast::channel(1024)` ring (~176 KB) per topic and re-creates them for
>   topics that are *already subscribed* — the carried set sat at five the whole
>   time. SQLite page cache is the second half. `pending_open_paths` is absent
>   from the profile, so the vendored patch is confirmed holding.
>   → `docs/measurements/2026-09-16-residual-leak-profiled.md`
> * **The +0.7 MB/min figure was wrong** — it is **+0.52**, and the sqlx pool
>   (~5 MB a connection) was the confound.
>   → `docs/measurements/2026-09-16-residual-leak-analysis.md`
> * **The soak is over.** It ended at t+371m because phone B left adb, not
>   because of the 6 h FGS kill; pid 6562 survived and was still running at
>   6 h 52 m.
> * **§3.3 is decided: the iroh comment will NOT be posted.** The draft stays as
>   a record.
> * **§3.4 is built and compiles, and has NOT been opened on a phone.**
> * **Ayni is now 0.1.7 (codes 81/82), built and signed.** Deliberately **not
>   tagged** — a `follower-v*` tag is itself the F-Droid release.
> * **The AAPS plugin now builds** (`BUILD SUCCESSFUL`, 14:45) and was never
>   built at the time this was written.
>
> * **PHONE B NOW HAS BOTH**: Ayni 0.1.7 and the AAPS plugin, installed 15:15,
>   both verified running and on screen. Phone B carries AAPS as well as Ayni —
>   loop disabled, virtual pump — which is what let the subject-side screens be
>   opened without going near the loop phone.
> * **§3.4 is DONE and was seen on a phone.** Opening it changed it: the
>   unknown-scope line wrapped to two lines under every reader and is now one.
>
> * **THE LOOP PHONE HAS TODAY'S WORK, INCLUDING THE ROTATION FIX** (installed
>   16:27; pump key intact, loop verified running afterwards).
>
> 🔴 **KNOWN GAP, ACCEPTED: followers cannot read 2026-09-16 after 15:34.**
> The first rotation ran unannounced (the bug fixed in `9ebbf69`), so today's
> epoch 20712 is sealed under a secret no follower holds. Repairing it needed a
> forced rotation before the day closed; that was **deliberately not done**,
> because the only route went through interrupting AAPS and the loop's
> continuity outweighs a day of dev/test sharing — see
> `loop-is-critical-sharing-is-not` in memory.
>
> **It self-resolves.** The next rotation (~15:34 on 2026-09-17) is announced by
> the fixed build, and that day's accumulated epoch is re-sealed under it. Only
> 2026-09-16 after 15:34 stays dark, permanently, and that is fine.
>
> ⚠️ **Do not "fix" this later by re-granting** — an existing member returns
> `AlreadyGranted` and publishes nothing. Closed days keep their own secret.

> §0's point about the loop phone is now closed.

Everything is committed and pushed. Working tree clean. Fifteen commits today.

⚠️ **`README.md` and `fdroid/nz.diaswarm.ayni.yml` have carried the owner's own
edits in the past — check `git status` before committing either.**

---

## 0. Read this first: what is running, and what is NOT installed

**Running unattended right now:**

| what | where | why |
|---|---|---|
| memory soak | phone B, `docs/measurements/2026-09-16-ayni-memory.txt` | validating the iroh workaround; **~6 h in** |
| burst alarm | this session's monitor | fires if native heap passes 600 MB |
| desktop peer | laptop, `~/.local/bin/diaswarm-peer` | **stock iroh on purpose** — the control arm |

🔴 **ALMOST NOTHING FROM TODAY IS ON A PHONE.** Built and tested is not shipped,
and this project has been caught by that distinction before.

| device | what is actually installed | what it is missing |
|---|---|---|
| **phone B** `2C01…YL` | Ayni 0.1.6, installed 08:00 today — **iroh patch only** | handles, carrying display |
| **loop phone** `2A28…AC` | AAPS from 2026-09-15 16:54 — **D32 network fix only** | iroh patch, rotation, scope picker, handle |

⚠️ **Installing Ayni on phone B ends the soak.** The two trade against each
other; that is why neither has happened.

⚠️ **The loop phone drives an insulin pump.** Its install is the owner's call,
every time, and phone B goes first.

---

## 1. The memory problem: one leak fixed, a second one found

**The big one was never ours.** `iroh` re-queues a failed QUIC path-open into
`pending_open_paths`, and the 333 ms retry calls `open_path_on_all_conns` for
each entry — which retries on *every* connection to that peer, each failing one
pushing its own copy back. One entry becomes C per tick. No dedup, no cap.

Measured on Android with heapprofd: **one 796 MB allocation**, 1.13 GB moved in
150 s, **net retained only 7.5 MB** — which is why summing net allocation finds
nothing and `max(size)` finds it at once.

**Upstream is aware and stuck.** [#4390](https://github.com/n0-computer/iroh/issues/4390)
open since July (milestone Sprint P, **overdue**), [#4509](https://github.com/n0-computer/iroh/issues/4509)
a duplicate with no labels. Three fix PRs exist: #4398 and #4414 closed *by their
own authors* — one saying *"without a clearer review path I do not want to leave
this open indefinitely"* — and #4522 open with merge conflicts and no review.
iroh 1.2.0 carries identical logic; upgrading does not help.

**Our workaround:** `vendor/iroh` — iroh 1.1.0 plus cbenhagen's dedup-and-cap
enqueue, verbatim so it lifts out cleanly. `[patch.crates-io]` in `diaswarm-net`
and `diaswarm-android`. See `vendor/iroh/DIASWARM-PATCH.md` for removal.

```
unpatched   348 MB at t+15m, 416 MB at t+54m (+127 MB in a minute), then killed
patched     263 MB at t+368m
```

🔴 **BUT THE SOAK FOUND A SECOND, SLOWER LEAK.** The patched build drifts
**+0.7 MB/min** over three hours (138 → 263 MB) — reaching 600 MB around t+14h
and ~1 GB in a day. Survivable where the original was not. **Not flat. The
memory problem is not solved.**

⚠️ **Three consecutive samples can look flat while the hour is climbing.** That
is how the residual was nearly missed. Read hours, not samples.

**Next step, and it is ready to run:** a heapprofd capture on the *current*
6-hour-old process. That is the condition every earlier capture failed — each
one was taken minutes after launch and misread SQLite cache warm-up as the
cause. The recipe is in memory (`iroh-pending-open-paths-leak`): phone B is
`userdebug` so heapprofd needs no `profileable` tag; `trace_processor` comes from
`get.perfetto.dev`; for symbols build with `CARGO_PROFILE_RELEASE_STRIP=none`
**and** a temporary `keepDebugSymbols`, then **install it** — an unstripped build
that is not installed symbolises nothing, because `strip=none` changes the link
and moves `.text`.

---

## 2. What shipped today

| | commit |
|---|---|
| iroh workaround vendored | `0bcdcca` |
| scoped grants — `grant_since` + spike | `7072e37` |
| subject handles — invite v4 | `a1d51af` |
| subject handles — both apps | `cd3f4ac` |
| daily secret rotation | `8988329` |
| Ayni shows what it carries | `b5d985b` |
| scope picker on grant | `cbfc379` |
| F-Droid: commit hash pinned | `d72b32b` |
| F-Droid: auto-update enabled | `d632b2d` |
| `roles.md`, `roles-to-product.md` | `649ca82`, `aee13ba`, `2b22d69`, `6bf76cb` |
| D31 header corrected | `5b6d0d1` |

**The three product changes had to land together** and none was any use alone:
`grant_since` could not express a window without rotation, and rotation was
invisible without a picker.

**Two new documents worth reading before touching any UI:**
[roles.md](roles.md) — the five actors, what each must see and what each may
never be told — and [roles-to-product.md](roles-to-product.md), the per-role
deltas with status.

---

## 3. Open, in the order I would take it

**Rewritten 2026-09-16 17:00.** Items 1–4 of the original list are closed; what
follows is what is actually left.

### Closed today

| | |
|---|---|
| ~~Profile the residual leak~~ | done — and it turned out there may be no residual leak. See §3a. |
| ~~Install today's work~~ | both phones. Loop verified running after. |
| ~~Post the iroh comment~~ | **decided: not posting.** Draft kept as a record. |
| ~~Readers' scope + withdrawal copy~~ | done, opened on a phone, and shortened because of what that showed. |

### 1. D29 step 1 — the desktop peer UI

**Unblocked, and the reason has nothing to do with clinicians.** D15 promises a
subject stays readable while their phone sleeps because somebody else holds the
bytes — and every holder today is a phone, subject to the doze and the ~6 h
foreground-service kill that this week has spent its time measuring. **One
mains-powered peer with a disk makes that promise structural rather than
probabilistic.**

It is a UI over crates that already exist, with no JNI, no NDK and no
cross-compilation: `diaswarm-peer` already runs headless on the laptop. Three of
five roles differ only in what they are granted, not in what they run.

### 2. D29 step 3 — the clinician gateway

🆕 **NEWLY UNBLOCKED, today.** D29 blocked this on time-scoped grants, quoting
the seam comment *"a grant reaches back over everything, and cannot be asked not
to."* `Vault::grant_since` plus daily rotation mean **"share the last 90 days"
can now mean 90 days** — so the screen D29 refused to let anyone build is
buildable and honest for the first time. Needs step 1 first; it is the same
screen at a different scope.

### 3. Prove the rotation fix on a device

`9ebbf69` is installed and **has never had a successful rotation run through
it.** The next one is ~15:34 on 2026-09-17. Watch for
`swarm: rotated the group secret` on the loop phone and for the follower's
`unreadable` count staying at 0. Until then the fix is tested only by
`rotation_reaches_readers.rs`.

### 4. Carried over from before today

* `peer-v0.1.0` can be re-tagged now the `macos-13` runner is fixed — four of
  five targets built last time.
* **The four-phone redundancy test remains the largest untested claim**, and
  there are two phones.

---

## 3a. The memory question, as it actually stands

**Do not describe this as "a leak" without reading
`docs/measurements/2026-09-16-leak-vs-fragmentation.md` first.**

* ✅ The **SQLite cap works** — +1.70 MB live per connection against a 2 MB
  ceiling, measured. Stop looking there.
* ✅ The **arena grows** at +0.51 MB/min (t=3.5), and **in-heap free space** at
  +0.28 (t=5.7). Both solid.
* 🔴 **A live leak is NOT proven.** +0.24 MB/min at t=1.6 is inside its own
  noise. Needs 4–5 h; a sampler has been running on phone B since 15:19.
* 🔴 **Fragmentation vs lazy decommit is unresolved** — `am send-trim-memory` is
  refused on a foreground service, so `malloc_info` is the remaining route.

⚠️ **+0.7 and +0.52 MB/min are both withdrawn.** So is "there is a residual
leak", pending the long run.

**Not doing, deliberately:** deleting the legacy bucket-carry in `share.rs`.
Establishing that no peer is still on the old topic costs more than the tidier
counter is worth, and the failure mode is a silent partition on the pump phone's
data path.

---

## 4. F-Droid

MR !48469 is **green and mergeable**, head `4bb1dc91`. Everything asked for is
done: screenshots moved to `en-US/images/phoneScreenshots/`, builds pinned to a
commit hash, auto-update enabled. Waiting on them.

⚠️ **The screenshots were the real blocker for four days**, not a stale label.
They sat one directory above where F-Droid reads, so the tooling never saw them
and `waiting-for-upstream` was correct the whole time.

---

## 5. Traps paid for today

* **Profile at 40+ minutes.** Every capture taken minutes after launch misread
  cache warm-up as the cause and cost a day on the wrong theory.
* **`max(size)`, not `sum(size)`.** The leak was one 796 MB buffer whose net
  retention was 7 MB.
* **Interleave A/B runs.** Measuring two configs half an hour apart read machine
  load as a regression and nearly reverted a good change.
* **Never conclude overnight behaviour from a run shorter than the claim.** Said
  on 2026-09-15, then broken the same day by calling a 15-minute flat window a
  fix that was gone within the hour.
* **Check how someone else's tool behaves before writing it down as fact.** Two
  comments asserted things about `p2panda-store` and `fdroid` that were wrong and
  took two minutes each to disprove.

**Five claims were withdrawn today or yesterday** — off-LAN proven, screenshots
uploaded, a connection leak, a page-cache fix, and a flat memory curve. The
pattern in all five is measuring for less time or less directly than the claim
needed.
