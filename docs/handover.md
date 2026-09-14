# Handover — 2026-09-15 07:30

Everything is committed and pushed. The only uncommitted files are
`README.md` and `fdroid/nz.diaswarm.ayni.yml`, which carry **the user's own
edits — do not commit them**.

---

## 1. What is running right now, and when it answers

**A soak on phone B, started 07:13.** It is the only thing with a clock on it.

```
scratchpad/fgs-soak.log      one line every ten minutes
```

**The question:** does Ayni's foreground service survive past **372 minutes**
(~13:25), which is where the previous build died. Anything before that proves
nothing.

Baseline at start: `pid 19707 · types=0x40000000 · effective=NONE`.

Phone B is screen-off and on USB. **Charging does not affect the thing being
measured** — the foreground-service time limit is not a doze mechanism — and it
isolates the variable. Unplug only if a combined doze test is wanted instead.

---

## 2. The finding this soak exists to verify

**Ayni could not watch overnight, and the cause was not doze.**

`dataSync` foreground services are time-limited by Android 15 to ~6 hours in 24.
At the limit the system kills the process and then **refuses every restart** —
the quota is spent. Measured:

```
21:25  service starts
03:37:54  ForegroundServiceDidNotStopInTimeException → process killed
          ForegroundServiceStartNotAllowedException: Time limit already exhausted
07:13  came back ONLY because the app was rebuilt and opened
```

**220 minutes down, and that number is an artefact** — Android resets the timer
when a human brings the app to the foreground, and nothing else was going to at
4 a.m. Unattended it is half a day.

**Doze was never the problem.** From deep IDLE at 21:40 until death at 03:37 the
follower held its two-minute cadence for **5 h 57 m**, standby bucket `10
(ACTIVE)`, `effective=NONE` throughout. The `DOZE|APP_BACKGROUND` flags that
appear afterwards are the *absence of a process holding a foreground service*,
not a cause. Two sessions were spent blaming doze; do not spend a third.

**The fix, already installed on phone B:** `specialUse`, which carries no such
limit. Verified in three places — source, binary (`0x40000000`, was
`0x00000001`), and the live service. `onTimeout()` implemented as a clean stop
in case a limit is ever applied to this type too.

**Nothing about it is proven until 13:25.**

---

## 3. Device state

| device | serial | state |
|---|---|---|
| **loop phone** | `2A281FDH2006AC` | AAPS up since 21:25, looping, sealing, publishing. **Drives an insulin pump — test on phone B first, always.** |
| **phone B** | `2C011FDH200MYL` / `192.168.88.213:5555` | AAPS + Ayni, both on the current build. Running the soak. |

The third adb entry is phone B again over wifi, **not a third device** — check
`ro.serialno` before believing otherwise. Unplugging phone B removes the USB
transport and leaves only the wifi one.

---

## 4. What landed over the last two days

* **CI**: `.github/workflows/crates.yml` — all five crates on every push to
  main. Before this, `diaswarm-net` and `diaswarm-android` had never run in CI.
  It caught a real breakage within hours.
* **`diaswarm-spaces` deleted** — crate, seven JNI entry points, Kotlin
  declarations, dead tests. `p2panda-spaces` and `p2panda-auth` are gone from
  both APKs.
* **D28** — the pool carries keys logs. Announcing and adopting ship together;
  announcing alone would publish the follower set.
* **D29** — the desktop is two products. `crates/diaswarm-peer` is the carrier
  half: works against the live pool, carried four strangers' subjects and 463
  operations it cannot read. Packaged for five platforms, tag `peer-v*`.
  **Never tagged — the first build for macOS and Windows will be the first.**
* **D30** — *if you read it, you carry it*. Already structural on the keys
  vault: a reader reads from its own store, and `carry` is the only thing that
  fills it. Reciprocity is mandatory, generosity is sizeable.
* **D31** — a rendezvous that depended on a locally-guessed pool size. Fixed
  with `pool::subject_topic`. Both phones are on the fix.
* **The loop phone's push reaches Ayni** — the open item two previous handovers
  led with. `live op from 9eeeac47` at 16:47.
* **Freshness is the sensor, not the transport**: `sensor 60s + drain 0.1s +
  transport ~11s`. The transport is the smallest and least variable term.

---

## 5. Open, in the order I would take them

1. **Read the soak at 13:25.** Pass → the overnight failure is fixed and Ayni
   can be released. Fail → see item 2.
2. **If it fails, the fallback is AAPS's approach**: lower `targetSdk`. AAPS
   runs at **32** and is thereby exempt from the entire FGS type-and-timeout
   regime — which is why the loop phone sailed through the same night. F-Droid
   has no targetSdk requirement, so it is available. It is a blanket opt-out
   whose scope is hard to enumerate, so it is a retreat, not a default.
3. **Tag `peer-v0.1.0`** if binaries are wanted. `fail-fast: false` is set
   deliberately: four of five targets working is more useful to learn than
   which one failed first.
4. **Delete the legacy bucket-carry** in `share.rs` once no peer remains on the
   old topic. `pushed` then becomes 1 and means something.
5. **Four phones.** D28's redundancy is laptop-proven and only two-phone
   demonstrated. The largest untested claim.
6. **The scoped-grant spike** (D29): filter a `SecretBundle`, `Dcgka::add` with
   it, assert the joiner opens epoch N and fails on N−1. Small, and it is the
   gate on the entire research/clinician path.

---

## 6. A spike somebody may want: AAPS at targetSdk 36

Not needed for diaswarm. Proposed because **diaswarm's two halves run under
different Android contracts from one codebase**:

```
publisher (AAPS plugin)  targetSdk 32   exempt from FGS types and timeouts
follower  (Ayni)         targetSdk 36   fully subject to them
```

That is why the same night killed one and not the other, and it means **a
background-behaviour lesson learned on the loop phone does not transfer to phone
B, or the reverse.** That trap is the reason this is written down.

Sized: only **two** AAPS services actually call `startForeground` —
`DummyService` (the persistent notification that keeps AAPS alive) and
`AlarmSoundService`. Two type declarations, two permissions. Then
`POST_NOTIFICATIONS`, then edge-to-edge, then unknowns.

**The interesting answer comes first**: does `DummyService` get time-limited?
Phone B only, and D9 says extend upstream rather than fork — so this is a spike
to measure and hand to AAPS, not a change to carry in the fork.

---

## 7. Traps, all paid for

* **Check the tool's own exit code, never a grep.** Three times in two days a
  green-looking signal was hollow: a `cargo test` that reported success from a
  trailing `echo`, one that ran in the wrong directory, and a gradle build
  declared "compiled" while it had failed on a missing NDK — leaving the
  previous night's APK in place with the old manifest. **Verify the artefact,
  not the source.**
* **Build Android through the scripts** — `./plugin/build-apk.sh`,
  `./follower/build-apk.sh`. Calling `./gradlew` directly fails on toolchain
  paths.
* **`adb logcat -d | grep | tail -1` is not the newest line.** The buffers are
  not merge-sorted. Sort on the timestamp, and bound queries with
  `logcat -T '<time>'` or yesterday's crash becomes today's alarm — both of
  which happened.
* **Every counter in the replicator counts events, not things.** `received` per
  store, `live` per p2panda increment, `sent` per topic. None bounds another.
  The store is the only thing that counts things.
* **Never conclude anything about overnight behaviour from a run shorter than
  the claim.** A 51-minute measurement and a 2-hour one both "proved" the
  foreground service was sufficient. A 6-hour limit cannot be seen in either.

---

## 8. Constraints

* `README.md` and `fdroid/nz.diaswarm.ayni.yml` have **uncommitted user edits**.
  Leave them.
* **No tags or releases** without the user saying so, including the F-Droid MR.
* `2A281FDH2006AC` drives an insulin pump. Phone B first. Always.
