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
cd ../aaps-diaswarm
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/28.2.13676358
./gradlew :app:assembleFullLoop -PappVersionSuffix=diaswarm
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
- **Nothing has run on a phone.** The module builds against a real AAPS
  checkout — `BUILD SUCCESSFUL`, all four flavours, with both `.so` files inside
  the AAR — and the JNI contract is exercised on the host by
  `crates/diaswarm-android/jni-test/run.sh`. That is further than it sounds and
  still not the same as working: nothing has loaded the library on Android,
  under Dalvik, in a process that is also running a loop.

## Verified so far

| | |
|---|---|
| Module compiles against AAPS | `:plugins:sync:swarm:assembleDebug`, 223 tasks, all four flavours |
| Native half packaged | `jni/arm64-v8a` and `jni/armeabi-v7a` inside the AAR |
| JNI symbols resolve | all eight, checked with `llvm-nm` and exercised from a JVM |
| Record logic matches `canon.py` | byte-identical over 19,129 real records |
| Rounding matches Python | ties-to-even; normalising a canonical stream moves nothing |
