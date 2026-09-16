# Handover — 2026-09-16 14:15

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

1. **Profile the residual leak** (§1). The soak has already told us it exists;
   more hours only refine the rate.
2. **Install today's work.** Phone B first for Ayni, then the loop phone for the
   plugin. **Rotation has never run on a device** — it fires once a day, so the
   first real one is tomorrow.
3. **Post the iroh comment.** Drafted at `docs/upstream/iroh-4390-comment.md`. Its
   value is no longer "a third platform" but *"three PRs exist, none merged,
   this needs a decision"*.
4. **Two small gaps** from `roles-to-product.md`: the readers list does not say
   what scope each reader got, and the withdrawal copy is still unwritten. The
   true sentence is in `roles.md` — *"they lose today and keep every day that
   already finished"*.
5. **D29 step 1**, the desktop peer UI. Three of five roles differ only in what
   they are granted, not in what they run.
6. **D29 step 3**, the clinician gateway — honest for the first time, because
   "90 days" can now mean 90 days.

**Not doing, deliberately:** deleting the legacy bucket-carry in `share.rs`.
Establishing that no peer is still on the old topic costs more than the tidier
counter is worth, and the failure mode is a silent partition on the pump phone's
data path.

**Also still open from before today:** `peer-v0.1.0` can be re-tagged now the
`macos-13` runner is fixed (four of five targets built last time), and the
four-phone redundancy test remains the largest untested claim.

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
