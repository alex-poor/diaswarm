# Handover — 2026-09-14, second session

The first session's handover is the commit message of `d052d76` and the entries
in `docs/migration.md`. This replaces it.

## What was asked

Four things, from a review of the project against its own goals:

1. put every crate in CI;
2. delete `diaswarm-spaces`;
3. close the keys vault's pool-redundancy gap;
4. update the loop phone and measure the publisher reaching Ayni.

## What landed

`388b48a`, `fc3fb75`, `620b173` on `main`. **Not pushed** — see Constraints.

| crate | result |
|---|---|
| diaswarm-core | 58 passed, 0 failed |
| diaswarm-keys | 27 passed, 0 failed |
| diaswarm-android | 16 passed, 0 failed |
| diaswarm-net | 45 passed, 0 failed |

From cargo's own exit code, written to a log by the process that ran it. Three
earlier runs in this session reported success that came from a trailing `echo`,
from the wrong working directory, or from a run aborted half way — which is the
same class of error as the counter that caused the first session, so the numbers
above are the ones that survived being checked.

### 1 · CI

`.github/workflows/crates.yml`. All four crates on push to `main` and on any PR
touching `crates/**`. **`ayni.yml` ran `cargo test` in `diaswarm-core` alone**,
on a tag or a follower-touching PR, so the transport crate had never run in CI
at all.

One job with one `CARGO_TARGET_DIR`, because there is no workspace and a job per
crate would build p2panda four times. `cancel-in-progress` so a second push
inside a minute does not pay twice.

### 2 · `diaswarm-spaces`, deleted

The crate, its seven `spaces*` JNI entry points, the Kotlin declarations in both
apps, `tests/replicate.rs`, `bin/twophone.rs`. `p2panda-spaces` and
`p2panda-auth` have left both APKs' dependency trees.

Nothing in Kotlin had called it since D26. The JNI guard listed all seven as
"dead by decision" — a holding pen that reads like a decision — and they shipped
to two phones in that state. Every property `tests/replicate.rs` asserted is
asserted by `tests/keys_replicate.rs` on the vault actually being cut over to,
which is why the test could go with the code rather than before it.

### 3 · The keys vault has a pool now — D28

`BucketMessage::HoldingKeys`, `Swarm::keys_wanted`, and a `keysCarryAll` that
adopts strangers heard in this phone's share and reports back what it holds so
the next tick announces it.

**The announcing half and the adopting half are one change deliberately.** A
peer saying "I hold keys-subject Y" while only followers hold Y has published
the follower set, which is the D18 leak. Strangers carrying it is what makes the
sentence ambiguous. Do not land one without the other.

Proven by `tests/keys_pool.rs`, mutation-checked both ways: removing the
announcement reproduces the exact prior symptom, silencing the carrier leaves
the pool with one findable copy.

**Two defects in this work were found by reading it, not by a test going red**,
and both are worth knowing about because neither was reachable at test-rig
scale:

* `tick` announced only into this peer's own buckets. A phone always carries its
  own keys log, but its own subject hashes wherever it hashes — usually *not*
  into its own share once a pool exceeds about four peers. The one peer that
  certainly has the data would have been the one peer that never said so.
  Invisible at two peers, where `REPLICAS` (3) exceeds the pool and everybody
  holds everything. `announce_buckets` is now a pure function with a unit test,
  because the rule cannot be expressed at the scale the rig runs at.

* `keysCarryAll` now takes an adoption budget, as `swarmTick` does. **Both apps
  pass `2`.** A first version had Ayni pass `0`, on the strength of a comment
  left in Ayni's tick by the session that fixed discovery — and the user
  corrected it: carrying other people's records is the entire point, and it is
  the app's name. `strings.xml` says so outright. A follower that adopts nothing
  keeps only the half of the bargain that benefits it, and it is the half that
  does not work — followers are most of the phones, so if they carry nothing the
  only peer holding a subject is the subject. See D28.

  **`an_app_in_the_pool_carries_a_share_of_it` now fails the build on a zero
  budget**, mutation-checked. Every existing guard was green while Ayni carried
  nothing for anybody, because they all ask whether a call happens and this
  defect lived in an argument.

### 4 · The loop phone — DONE, and the push is delivered

* ✅ Built from `fc3fb75`, installed on **phone B**, signer verified matching.
  Healthy: native library loads, `swarm: keys carrying 1 log(s)` — which is the
  live proof that `keysCarryAll` returns through its **changed 4-argument JNI
  signature**, the failure that would otherwise appear at load time.
* ✅ **The loop phone was updated at 16:44** — signer matched, pump key
  survived, rollback saved, and the loop kept running throughout (a Libre 3
  reading of 182 landed at 16:45:27, drained, sealed and pushed). The install
  needed a permission rule the user added mid-session; three earlier attempts,
  including a read-only `apksigner`, were refused by the auto-mode classifier
  while the identical command for phone B went through.

* ✅ **Ayni rebuilt and installed on phone B at 16:45**, carrying D28 and the
  new adoption budget.

* ✅ **THE MEASUREMENT IS MADE.** Both handovers led with this item.

  | time | peer | line |
  |---|---|---|
  | 16:44:23 | loop phone, pre-fix | `shadow agrees — … failures 0` — no `pushed` field |
  | 16:45:27 | loop phone, fixed | `… failures 0, **pushed 1**` |
  | 16:46:41 | loop phone, fixed | `… failures 0, **pushed 2**` |
  | 16:47:47 | Ayni, phone B | `sync: **live op from 9eeeac47** n=1` |

  `9eeeac47` is the loop phone. Two independent sources — the publisher's own
  count of `publish` returning, and p2panda's per-arrival peer attribution on
  the follower. And `keys holders for 552f688a: 1 (9eeeac47)` on the same pass,
  which is D28 working on hardware.

  **What this does not establish:** freshness. The 16:47:47 arrival is two
  minutes after the follower process started, which is discovery on a cold
  process, not push latency. Nor does it establish that every push lands, or
  anything off-LAN, or anything overnight.

## The counter, and what replaced it

**The live counter was wrong a fifth time — in its units, not its arithmetic.**
`live_received()` sums `Metrics::received_live_operations`, which does not
advance once per operation (`n=2` per event on one build, `n=1` on another, so
the multiplier is not even constant), while `received` is one per stored
operation. Printed side by side they produced `received=20401 live=21622` on a
phone: more pushed arrivals than arrivals, the shape versions 2 and 3 were
rejected for, reached this time by honestly reporting a number that is not the
same quantity.

**Renamed, not fixed:** `counts stored=N live_raw=M (not comparable)`. Four
attempts to *derive* the right number were each wrong and dividing by an
unmeasured multiplier would be a fifth. `live_raw > 0` means pushes are
arriving; its magnitude means nothing.

**What to use instead**, and this is the part that matters: the per-arrival
`live op from <peer>` lines, and the freshness series below. Both name a thing
that happened rather than summarising things that happened.

## Freshness: measured, and the transport is the smallest term

The question every counter was a proxy for. It needed **no new code** —
`SyncWorker` already logged `keys newest for <subject>: Ns old`.

Raw series on phone B, settled window, 13 samples over 22 minutes: min 23s,
median 24s, max 107s, all under the 120s poll interval.

**But the raw age is not transport latency**, and separating them is the actual
result. With the loop phone's `sealed epoch` times beside it:

```
sensor 60s cadence  +  drain ~0.1s  +  transport ≤1 seal cycle, usually ~11s
```

* the **drain** adds 74–228 ms — it seals on the reading;
* the **60s cadence is the Libre 3 itself** (59.6–60.8s inter-arrival measured);
* a 171s "seal stall" was a **183s sensor gap** — no reading arrived at all,
  then five backfilled at once. The publisher had nothing to seal;
* the **bimodal 23/83s split** is exactly one seal cycle: the follower is either
  current or one seal behind.

**The transport was the entire subject of two sessions and is now the smallest
and least variable term.** What a follower shows is dominated by the sensor,
including its multi-minute gaps, which nothing in this repository can shorten.

The consequence is for wording rather than code: a follower showing "3 minutes
ago" during a sensor gap is telling the truth, and showing the age is what lets
a person tell an old reading from a broken network.

## Where the devices actually are, 2026-09-14 21:30

**All three on the D31 build**, installed in that order, signer verified on both
phones, pump key intact, rollback saved.

| | state |
|---|---|
| loop phone `2A28…AC` | publishing · `pushed 2` · `pool 4 peers` · looping throughout |
| phone B AAPS `2C01…YL` | in the pool, carrying |
| Ayni (phone B) | `pushed=yes` · newest reading **24s old** · holder `9eeeac47` |

`pushed 2` is the subject topic plus the legacy bucket topic, which is the
transition working. It is **not** a count of sends — see D31.

The relabelled counter reads as intended, and is worth seeing once:

```
sync: counts stored=16 pushed=yes (p2panda's raw counter 5, not a count of anything)
```

Under the old label that was `received=16 live=5`, which invites "31% arrived
pushed" — a sentence about two different units.

**And a laptop was visible as a holder.** Minutes before the restart Ayni
reported `keys holders for 552f688a: 3 (9eeeac47, b8e0c9ba, 122dc922)`, the last
being `diaswarm-peer` run from this machine. A desktop carrying a phone's sealed
records, offered to a follower as a fallback source — D15, D28 and D29's carrier
composing on real hardware for the first time.

## The overnight soak

Phone B is to be left unplugged and untouched. The monitor samples every five
minutes into `scratchpad/doze-soak.log` over the **wifi** transport
(`192.168.88.213:5555` — the USB one goes with the cable) and speaks only on a
doze transition, on Ayni becoming network-restricted, or on true staleness over
900s.

**Before it was interrupted for these installs it had already beaten the
previous best.** 17:46→19:46, twenty-four samples:

| | |
|---|---|
| deep IDLE | **2 hours uninterrupted** (previous best: 51 min) |
| network-restricted | **0** — the actual overnight failure mode, never seen |
| true staleness | min 32s · median 114s · max 247s |
| over 10 min stale | **0** |
| wake cadence | stretched 120s → **4–6 min**, never stopped |

The median of 114s against a 4–6 minute wake cadence is the push working:
operations arriving between wakes rather than being waited for.

⚠️ **What is still unproven is the long idle windows.** Doze starts at an hour
and grows toward six, and the failure that was never reproduced under known
conditions — relay gone by morning — lives out there. That is what tonight is
for.

**Reading it tomorrow:** `true_age` already includes time elapsed since the
follower last logged, so a worker that stopped shows as growing staleness rather
than a frozen reassuring number. `doze=` and `effective=` are the two columns
that matter; `effective` anything but `NONE` is the known failure.

## Still open

1. **The overnight soak is running and unanswered.** That is the headline open
   item and it resolves itself by morning. Read
   `scratchpad/doze-soak.log`; the evidence format that worked before is
   "N minutes in deep IDLE, M of M samples fresh, effective=NONE throughout".
2. **The pool has only ever been three phones.** Peers carrying genuinely
   disjoint shares, and a stranger adopting unasked, are proven in
   `tests/keys_pool.rs` on a laptop and **not** on hardware — that needs four or
   more devices. [D28](decisions.md) works on two phones, which demonstrates the
   mechanism and not the redundancy.
3. **The relabelled `counts` line is committed but not installed** on either
   phone, deliberately: reinstalling resets the soak. It rides along with
   whatever is next built.
4. **`live_raw` may not deserve to exist.** Now that `live op from <peer>` and
   the freshness budget answer the real questions, a counter that has misled
   five times and needs a disclaimer inside its own label is a candidate for
   deletion rather than maintenance. A decision, not a bug.
5. **Nothing is measured off-LAN on the fixed build.** The relay path was proven
   on 2026-09-14 for the *core* vault; the keys vault's live push has only ever
   been seen on one wifi.
6. **`event` record kind is emitted and nothing reads it.**
7. **The publisher's own freshness is now the dominant term**, and it is the
   sensor's. Nothing in this repository can shorten a 183-second Libre 3 gap;
   what it can do is never imply the number is fresher than it is.

## Next: the desktop, and what it is blocked on

[D29](decisions.md) settles that "a desktop app" is two products.

**The desktop peer is unblocked and worth building on its own merits** — a
reader plus an always-on carrier, over crates that already exist, with none of
Android's difficulty. It repairs the pool's structural weakness, which is that
every holder today is a phone that sleeps.

**The research/clinician gateway is blocked on time-scoped grants** — and the
blocker is smaller than `Vault::grant`'s seam comment concluded. Reading
`p2panda-encryption` 0.7.1: `EncryptionGroup::add` hardcodes the full bundle,
but `Dcgka::add` one layer below **takes the bundle as a parameter**, and
`SecretBundle::from_secrets` + `GroupSecret::timestamp()` are both public. So
scoped grants look reachable on the pinned version with no fork.

⚠️ **Read from source, not built.** The spike that settles it is small: filter a
bundle, `Dcgka::add` with it, assert the joiner opens epoch N and fails on N−1.
Do that before anyone designs a screen that says "the last 90 days".

## Constraints that still apply

* **Nothing is pushed.** Three commits sit on local `main`. Pushing runs CI, and
  the user asked for that to be deliberate rather than incidental.
* **No tags, releases, or changes to the existing F-Droid MR until the user says
  so.** `fdroid/nz.diaswarm.ayni.yml` and `README.md` have uncommitted user
  edits — do not commit them. They were carefully excluded from all three
  commits above.
* `2A281FDH2006AC` is the **live loop phone** driving an insulin pump. Test on
  phone B (`2C011FDH200MYL`) first, always. The third adb entry,
  `192.168.88.213:5555`, is phone B again over wifi — not a third device.
