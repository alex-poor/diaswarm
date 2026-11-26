# Migrating the live loop to the p2panda 0.8.2 stack — a runbook

**Status: NOT YET RUN.** The loop currently runs a working swarm publisher on the
pre-migration (0.7.1-era, vendored forks) build. Everything below is staged in
PRs #18–#23 but nothing has been installed on the loop. This is the deliberate,
loop-critical rollout — it is **not** a casual "test the prune" install, because
putting the APK on the loop migrates the live publisher wholesale: 0.7.1 → 0.8.2,
restream removed, WAL, the catch-up affordance, and D35 pruning, all at once, and
asks the loop's **existing** keys vault to come across the migration.

The loop's own dosing is never the subject here: the swarm plugin is read-only
isolated (D7), so the worst case is a lost day of *sharing*, not a stopped loop.
But treat it as loop-critical anyway.

## 0. Prerequisites
- PRs #18–#23 reviewed (ideally merged, so the installed build == a known tag).
- The APK is built from the **same AAPS base the loop runs** — verified
  2026-11-26: r8-validate base = fork/main HEAD = `09d216eea2` = the loop's
  running base. No loop fix is rolled back; the APK only *adds* the swarm/0.8.2
  changes. Re-verify this on the day (fork/main can move).
- A phone-B follower on 0.8.2 is already installed and working.

## 1. Back up first (both halves)
- **Rollback APK:** `build-loop-apk.sh` saves one to `~/projects/camaps/apk/rollback/`.
  Confirm the file for the CURRENT installed build exists before installing.
- **The loop's keys vault:** pull the swarm vault DB + `-wal` off the loop
  (rooted) so a broken migration can be restored. The subject's *own* glucose
  history is in AAPS's own DB and is not at risk; this is the shared vault only.

## 2. Think before shipping
Answer, before installing: does the 0.8.2 build read the loop's EXISTING 0.7.1
vault cleanly? The migration spike proved the `AnyOperation` read-path
round-trips on FRESH data; a live vault with real grants, epochs and history is
the untested case. WAL also converts the existing DB file on first open. These
are the real risks — not dosing.

## 3. Install (gated on an explicit go)
- The APK signer matches the loop's installed signer (verified), so `adb install -r`
  is an update that preserves `/data` and the pump key.
- `adb -s <loop> install -r <apk>` → launch → **`pidof` to confirm AAPS came back**
  (install -r leaves it dead) → confirm the loop reconnects to the pump and a
  cycle runs. THIS is the loop-critical gate: if dosing does not resume, roll back
  immediately.
- Then confirm the swarm publisher starts on 0.8.2, opens the existing vault, and
  serves (no "unreadable" storm, grants intact).

## 4. Validate the migration's sharing + the D35 prune
- A fresh follower (phone B, re-onboard) completes WITHOUT the OOM, and shows
  recent readings — this is the prune's on-device proof.
- Watch a rotation fire (`rotateIfDue`, daily) → `swarm: pruned N old segment(s)`
  in the log → the loop's shared vault is bounded.
- Confirm `SEGMENT_KEEP=2000` bounds a fresh follower's ingest to a safe peak on
  the real device; tune (or move to an epoch-exact window) if needed.
- Re-check revocation / re-admit (D13) still hold with pruning on.

## 5. Rollback triggers
- Dosing does not resume, or the pump will not reconnect → `install -r` the saved
  rollback APK at once.
- The vault fails to open / sharing is dark and does not self-heal within a
  rotation → restore the vault backup, or roll back the APK; a lost day of
  sharing is acceptable, a wedged publisher is not worth leaving on the loop.

## Why this order
The prune is already proven off-loop (`ingestprobe`: 500 ops at 50 MB vs
unbounded; `wire::prune_segments` unit test). So the on-device work is really
"migrate the loop to 0.8.2 safely", and the prune validation rides along for
free. Do it as its own event with backups — not to tick a box.
