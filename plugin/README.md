# The AAPS plugin

A `DataSyncSelector` sibling of `plugins/sync/xdrip`, kept in this repository
rather than in the AAPS tree.

## Why it lives here

The native symbol names are derived from `SwarmNative`'s package and class name,
so the Kotlin and the Rust are one contract. Renaming either half breaks the
other **at load time on a phone**, not at compile time on a desktop. One
repository makes that a single commit that fails loudly.

The same reasoning runs through the whole module: nothing with a decision in it
is written twice. The record shape, canonical encoding, rounding, deduplication
and epoch arithmetic are all in [`crates/diaswarm-core`](../crates/diaswarm-core),
which is asserted byte-identical to `tools/canon.py` over 19,129 real records.
Kotlin owns only what Rust cannot see: the typed `DataPair`s, the `isValid`
check, and this plugin's own high-water marks.

## Wiring it into an AAPS checkout

**The checkout this is wired into builds a live closed loop**, and more than one
session works in it at once. So the wiring lives on a branch **in its own git
worktree**, never in the working tree that builds the pump APK:

```sh
git worktree add ../aaps-diaswarm diaswarm-addon
./plugin/build-apk.sh            # build only
./plugin/build-apk.sh --install  # build, verify the signer, install, relaunch
```

**USE THE SCRIPT. THE BARE `gradlew` LINE THAT USED TO BE HERE IS A TRAP**, and
it cost most of a session. It named `ANDROID_NDK_HOME` and nothing else, which
reads as though the rest of the toolchain is whatever the machine has. It is
not:

| | where it actually is | what the machine's default gives you |
|---|---|---|
| JDK | `$CAMAPS/tools/jdk-21.0.5+11` | the system JDK, currently 26 |
| SDK | `$CAMAPS/tools/android-sdk` | `~/Android/Sdk`, a different install |
| NDK | under that SDK | a different NDK, if any |

AAPS builds against `compileSdk 36`, and JDK 26 cannot run the
`androidJdkImage` transform over `core-for-system-modules.jar` — `jlink` fails
inside `:core:ui:compileFullReleaseJavaWithJavac`, in an AAPS module, with an
error that looks nothing like "wrong JDK". `build-apk.sh` sets all three, and
delegates to camaps' `build-loop-apk.sh`, which owns the parts that matter: it
**refuses to install if the signing certificate no longer matches the device**,
which is the check protecting the pump key, and it saves a rollback APK.

To compile just this module without packaging an APK — enough to check the
Kotlin — the same environment applies:

```sh
export JAVA_HOME=$CAMAPS/tools/jdk-21.0.5+11 PATH="$JAVA_HOME/bin:$PATH"
export ANDROID_HOME=$CAMAPS/tools/android-sdk
export ANDROID_NDK_HOME=$(ls -1d $ANDROID_HOME/ndk/*/ | sort -V | tail -1)
cd $CAMAPS/sdk/aaps-diaswarm && ./gradlew :plugins:sync:swarm:compileFullReleaseKotlin
```

**A branch is not enough on its own, and this was learned the hard way.**
Uncommitted changes are not on a branch — they are in the working tree, and the
working tree is shared by every branch that tree checks out. Wiring committed to
no branch was carried onto `main` by someone else's `git checkout`, which is
precisely the thing branching was supposed to prevent. A worktree isolates the
files as well as the history; commit early there, and never leave that repository
dirty.

Verified both ways: the `main` build produces an APK with no swarm classes, no
`libdiaswarm` `.so` and zero occurrences in the dex; the branch build produces
one containing `lib/arm64-v8a/libdiaswarm_android.so` and all five plugin
classes, signer unchanged so `install -r` stays an update.

`buildRustCore` cross-compiles the core for `arm64-v8a` and `armeabi-v7a` and
copies the `.so` into `src/main/jniLibs`, where the library packages it from.
The `.so` is **not committed**: a checked-in binary is a second, opaque copy of
the frozen spec that nobody can diff and everybody trusts.

## Reaching each other

Peers meet through an iroh relay (decision D23), so a follower works from
anywhere rather than only on the subject's wifi. The relay never sees anything
it can open; it exists so two phones behind NAT can find a direct route, and
drops out of the path once they have. It defaults to `n0`'s public
Asia-Pacific relay and is meant to be replaced with your own.

Before 2026-09-11 no relay was configured and discovery was mDNS only, which
meant the whole thing worked on one wifi and nowhere else.

## Following somebody: the follower is a different app

**Install `aapsclient`, not `full`.** Putting a followed person's glucose on the
AAPS graph means writing into AAPS's own `glucoseValues` table, and in the app
that drives a pump that is not a display bug — `CompatDBHelper` fires
`EventNewBG`, which extends `EventLoop`, and `InvokeLoopWorker` doses on it. So
[`SwarmFollowerBg`](src/main/kotlin/app/aaps/plugins/sync/swarm/SwarmFollowerBg.kt)
refuses unless `Config.AAPSCLIENT`, and that is a build flavour: no preference,
no import and no bug in this module can turn it on. See decision D22.

The flavour is also the right shape for a follower. Its `applicationId` is
`info.nightscout.aapsclient`, so it installs **alongside** the loop app — on the
same phone if you like — with its own database, and `Config.PUMPDRIVERS` is
false, so it has no pump driver compiled into it at all.

```sh
cd ../aaps-diaswarm
./gradlew :app:assembleAapsclientLoop -PappVersionSuffix=follower
adb install -r app/build/outputs/apk/aapsclient/loop/app-aapsclient-loop.apk
```

Then, entirely on the two phones — no adb, no root:

1. **Follower** · AAPSClient → Config Builder → enable **Swarm** → its gear icon
   → *Show my invite*. A QR code appears. The follower app has its own identity;
   the loop app's grants do not carry over to it.
2. **Subject** · AAPS → Swarm → *Scan a code* → scan that QR → **Share with
   them**. The grant is written on the next sync pass.
3. **Follower** · Swarm → *Scan a code* → scan the subject's invite → **Follow
   them**.
4. Wait two minutes. The readings land on the follower's overview graph.

*Show on the glucose graph* only needs setting when the follower follows more
than one person — AAPS has one glucose series and somebody has to say whose it
is. Following exactly one person needs no choice and asks for none.

The first pass reaches a day back and no further: a grant may carry years, a
graph shows hours, and decrypting a year to draw six of them is how a follower's
first impression becomes a frozen app.

## What is not done

- **`publish()` does nothing.** The plugin canonicalises and counts; it has no
  swarm to publish to yet. Sealing is `tools/seal.py` (the reference) and
  `spike/p2panda-seal` (what the shipping key layer does), and neither is wired
  in. A plugin that pretended to publish would be worse than one that says it
  does not.
- **`ProfileSwitch` is not mapped.** §2 requires ISF and target blocks
  normalised to mg/dL, because AAPS stores profile blocks in whichever unit the
  user set while glucose values and temporary targets are always mg/dL.
  Emitting one un-normalised would put the same quantity in one stream a factor
  of eighteen apart. It needs the block API read properly first.
- **No screen.** Granting is driven by two preferences —
  `swarm_grant_reader` and `swarm_revoke_reader` — acted on next sync pass and
  then cleared. That is where a settings screen would write, so the mechanism is
  the lasting part; until one exists they are set with `adb`. The endpoint id and
  subject key are written to the log because there is nowhere else to see them.
- **`follow` only.** `clinician` and `cohort` are separate key trees (§7.2) and
  want a UI that says which one you are handing over. Offering them through a
  preference nobody can see would be worse than not offering them.
- **Pull, not push.** A reader fetches from the phone, so the phone has to be
  awake and reachable. The 3 a.m. low alarm — the flagship — needs push or
  store-and-forward, and neither exists. §9.5's Android background limits are
  named in the docs and still unmeasured.

## It has run

On a live closed loop (Pixel 7, YpsoPump, HovorkaMPC), 2026-09-09. The plugin
was enabled, sealed the phone's own history, and the vault was pulled and opened
on a laptop with a granted key:

```
swarm: sealed epoch 20705, 1129 records
swarm: 73 epochs 20630..20705, 0 grants
swarm: 1 post-emit edits this run

  grant   for follow from segment 0    73 wraps written
  opens   73 of 73 epochs, 33,278 records
  stop    from segment 73 — immediate, the open segment was closed first
  opens   73 of 74 epochs                ← the day sealed after the stop is dark
```

The vault carried `"offset": 43200000` — the device's own standing +12, derived
on the phone. **The amendment counter fired once on real data**, which is the
§7 measurement of post-seal edits beginning rather than being estimated.

## Verified

Installed on a live closed loop (Pixel 7, YpsoPump, HovorkaMPC), 2026-09-09.

| | |
|---|---|
| Record logic matches `canon.py` | byte-identical over 19,129 real records |
| Rounding matches Python | ties-to-even; normalising a canonical stream moves nothing |
| JNI symbols resolve | all eight, `llvm-nm` and exercised from a JVM |
| Builds into a loop APK | verified against a personal AAPS build |
| Signer unchanged | `install -r` stayed an update; app data and pump pairing survived |
| **AAPS registered it** | `ConfigBuilder_Enabled_SYNC_SwarmPlugin:false` at startup |
| **…and hid it** | `ConfigBuilder_Visible_SYNC_SwarmPlugin:false` |
| **Native library loads only when enabled** | `nativeloader: Load …libdiaswarm_android.so … ok` appears once, when the worker first runs. **Not** `/proc/<pid>/maps`: the `.so` is packaged uncompressed and loaded from inside the APK, so it never appears there by name — an earlier check that read zero would have read zero either way |
| Loop unaffected | `Closed Loop · looping`, the APS still deciding, pump pairing intact, AOT-compiled |

The library ships inside the APK and is never mapped into the process. That is
the claim `enableByDefault(false)` was making, and it is now measured rather
than reasoned.
