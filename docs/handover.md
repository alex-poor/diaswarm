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

## What this session found and did not fix

**The live counter is wrong a fifth time, in its units.** Version 5 is right to
report p2panda's number and derive nothing. But `live_received()` sums
`received_live_operations`, which `migration.md` already records as advancing
**two per event**, while `received` is one per stored operation — and they are
printed on one line under one word:

```
sync: counts received=15632 live=15531        # phone B, 16:32, Ayni 0.1.5
```

Read naively that says 99% of arrivals were pushed. It does not say that and
cannot. It also disagrees with the negative control recorded earlier the same
day (`received=4 live=0`, called "the right answer") and nobody has established
which reading is the surprising one. Left alone deliberately: four attempts to
be clever about this counter were wrong, and a fifth made in passing while
changing something else would be the same mistake. `docs/migration.md` has the
detail.

## Still open, from before

1. **Ayni has not been rebuilt**, so the installed 0.1.5 still carries nothing
   for strangers and reports no `keys holders for <subject>: N`. Each app
   bundles its own `.so`, so 0.1.5 is not broken by the signature change — it
   simply does not have any of this. Rebuilding it is what makes the pool
   bigger than one carrier:

   ```sh
   ./follower/build-apk.sh --install     # phone B first
   ```
2. **De-duplication makes pushes invisible when catch-up wins.**
3. **No overnight soak** — hours, never a night.
4. **`event` record kind is emitted and nothing reads it.**
5. **The pool has only ever been three phones.** Disjoint shares and a stranger
   adopting unasked are tested on a laptop and unproven on hardware.

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
