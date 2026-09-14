# Handover — transport work, 2026-09-14

## What was asked

1. Write network tests covering realistic transport scenarios (doze, LAN→off-LAN
   switch, restarts, outages) so transport regressions get caught, not rediscovered.
2. Find and fix why the follower was minutes behind.

## What was found

**p2panda's live mode has a send half that was never wired.** `LogSync` catches a
peer up and then switches to live mode, where new operations are pushed over
gossip via `SyncHandle::publish`. Nothing in this project ever called it:
`replicate.rs` moved the handle into its spawned task as `let _keep = handle`
and only ever subscribed. Operations went to SQLite, which is not on the network,
and reached a follower whenever the next catch-up sync happened to run.

That is why freshness always tracked the sync interval rather than the publish,
and why doze, a WifiLock, the relay and the subscription lifetime each looked
like the cause in turn. Measured before the fix: `received_live_operations: 0`
in all 69 live-mode sessions ever observed.

## State of the work

Commits `634acaf` … `606ba00` on `main`. All tests green:

| crate | result |
|---|---|
| diaswarm-core | 58 passed, 0 failed |
| diaswarm-keys | 27 passed, 0 failed |
| diaswarm-android | 16 passed, 0 failed |
| diaswarm-net | 50 passed, 0 failed |

### The fix

* `crates/diaswarm-net/src/replicate.rs` — the `SyncHandle` is kept in a
  `handles` map instead of buried in the task; `broadcast(subject, op)` publishes
  on every topic `carry` recorded for that subject.
* `crates/diaswarm-android/src/lib.rs` — `KeysVault` holds a clone of the pool's
  replicator; every `wire::publish` / `publish_control` site pushes what it wrote.
* Both apps — `keysSealChecked` reports `pushed=N`; `keysSyncEvents` leads with
  `counts received=N live=M`; the plugin's pass line carries `pushed`.

### Tests added

* `crates/diaswarm-net/tests/live_mode.rs` — 4 tests: a segment published while a
  peer is listening arrives pushed; so does a grant; broadcasting an uncarried
  subject is 0; and pushing survives a `restream()`.
* `crates/diaswarm-net/tests/two_process.rs` — a fourth test crossing a real
  process boundary, plus a check that the follower cannot report more live
  arrivals than the publisher sent.
* `crates/diaswarm-net/src/bin/keyspeer.rs`, `keyswatch.rs` — a publisher/follower
  pair and a laptop watcher, both usable by hand.

All mutation-checked: removing the `publish` call makes them fail with the exact
hardware symptom (catch-up works, live delivers nothing).

Pre-existing flake fixed: the two-process tests contended when run in parallel
(two consecutive runs failed two *different* tests; serial passed 4/4 twice).
They now take a static lock.

### Incidental fixes

* The sync event log was unbounded — ~300 bytes per `SyncFinished`, several per
  session, every couple of minutes. Capped at 256. That app has been OOM-killed
  before, and when it is killed the loop stops.
* `swarm: keys subject <hex>` now logged at plugin startup. It is a different key
  from the core vault's and was previously unreadable from a phone.
* `follower/build-apk.sh` added — the Ayni build recipe was re-derived by hand
  each time.

## Evidence, and its limits

Live delivery is confirmed on hardware, from p2panda's own `Metrics` rather than
from any counter in this repo:

| peer | build | sessions | `received_live_operations` |
|---|---|---|---|
| `9eeeac47` — loop phone | pre-fix | 175 | `0` in every one |
| `b8e0c9ba` — phone B's AAPS | fixed | 55 | `0` |
| `b8e0c9ba` — phone B's AAPS | fixed | 1 | **`7`** |

Also, with per-arrival peer attribution turned on, **every** live arrival came
from the peer running the fix; none from the pre-fix loop phone, none from a
follower that publishes nothing.

**Not verified: the loop phone's publisher reaching Ayni.** That needs the loop
phone updated, which was left to the user. The APK is built and the signer
verified as matching:

```
adb -s 2A281FDH2006AC install -r \
  /home/alex/projects/camaps/sdk/aaps-diaswarm/app/build/outputs/apk/full/loop/app-full-loop.apk
```

## Where I wasted the most time, so you don't

I wrote a `live` counter to measure the fix and got it wrong **five times**.
Every version tried to derive a per-operation fact ("was this one pushed?") from
`Metrics::received_live_operations`, which is cumulative over a session. Each
over-reported; two were arithmetically impossible (`live` > `received`):

| version | rule | reported |
|---|---|---|
| 1 | `received_live > 0` | 1,052 live of 1,059 |
| 2 | delta keyed on `session_id` | 17,128 of 8,103 |
| 3 | delta keyed on `(topic, peer, session)` | 5,245 of 3,322 |
| 4 | boolean: did the counter change? | 8,894 of 8,915 |
| 5 | **report p2panda's number, derive nothing** | current |

The decisive measurement was making the code print the peer and raw value behind
each arrival: the counter advances `n=2,4,6,8,…`, two per event the stream
observes, so no per-event rule can be right in either direction. `live_seen` now
holds each open session's counter as p2panda reported it, `live_retired`
accumulates ended sessions, and `live_received()` adds them.

The lesson, if it's useful: a derived diagnostic whose failure mode is
over-reporting gets believed, because it agrees with what you hoped. I believed
three of them. Assert against an independent source — here, the publisher's own
`pushed=` count — not against plausibility.

## Open items

1. **The loop phone is not updated.** Only way to prove the path that matters.
2. **The keys vault has no pool redundancy.** `keysCarryAll` carries only the
   phone's own subject and the ones it follows; `keysCarry` is in the JNI guard's
   known-dead list as superseded. So D15 — any holder serves identical bytes, so
   a sleeping subject stays readable — holds for the core vault and **not** for
   the vault being cut over to. This is a decision about what the cutover means,
   not a bug in the above.
3. **De-duplication makes pushes invisible when catch-up wins the race.** p2panda
   drops a live operation whose hash is already in the session's dedup buffer.
   On a quiet LAN with sessions re-syncing every few seconds, catch-up often
   wins. So `live` measures pushes that *beat* catch-up, not pushes sent.
4. **Real overnight soak not yet run** — only hours, never a night.
5. **`event` record kind is emitted and nothing reads it** (documented gap).

## Constraints that still apply

* **No tags, releases, or changes to the existing F-Droid MR until the user says
  so.** `fdroid/nz.diaswarm.ayni.yml` has uncommitted user edits — do not commit.
* `README.md` has uncommitted user edits — do not commit.
* `2A281FDH2006AC` is the **live loop phone** driving an insulin pump. Test on
  phone B (`2C011FDH200MYL`) first, always.

## How to run things

```sh
# tests (per crate — there is no workspace root)
cd crates/diaswarm-net && cargo test

# build + install
./plugin/build-apk.sh [--install]      # AAPS publisher plugin
./follower/build-apk.sh [--install]    # Ayni follower

# watch one subject from a laptop (joins the REAL pool; adopts nothing)
cd crates/diaswarm-net
./target/debug/keyswatch <dir> <subject-hex> <seconds>

# the subject hex is now in the plugin's startup log:
adb logcat -d | grep "swarm: keys subject"
```

The running record of every finding and correction is `docs/migration.md`.
