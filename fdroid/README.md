# Submitting the follower to F-Droid

`nz.diaswarm.ayni.yml` is the build recipe. It belongs in
[fdroiddata](https://gitlab.com/fdroid/fdroiddata) as
`metadata/nz.diaswarm.ayni.yml`, submitted as a merge request. It lives here
so the recipe that produces a published binary is versioned beside the source it
builds.

## What is published, and what never will be

The **follower** — a viewer for somebody else's glucose, with no loop code in
it. The **AndroidAPS add-on** that publishes a loop's records is not distributed
here or anywhere: running a loop means assembling a medical device yourself.
That split is decision [D24](../docs/decisions.md).

## Why this is one repository

It was two until the follower stopped being a flavour of AndroidAPS. Building it
needed the AAPS fork as a `srclib`, a committed `iconify.aar` inherited from
upstream that F-Droid's scanner rejects, and a `versionCode` shared with the
loop app. The follower now contains no AAPS at all, so none of that applies: one
`Repo:`, one `subdir:`, no `srclibs:`.

## What F-Droid has to do that is unusual

Install a Rust toolchain. The record core — canonical form, sealing, the vault,
the network — is Rust, compiled into the `.so` the app loads. It is built from
source during the build and **never committed**: a checked-in binary would be an
opaque second copy of a frozen specification, and the scanner would rightly
reject it.

`rustup` is available through `sudo:` in the buildserver; Fennec uses it the
same way. That was the one part of this nobody here had confirmed, and it is
confirmed by their own metadata rather than by hope.

## Releasing

1. Bump `versionCode` and `versionName` in `follower/build.gradle.kts`.
   `versionCode` there is a **base**, not a shipped number: the build multiplies
   it by ten and adds one per ABI. It must only ever increase; it is the
   follower's own sequence and has nothing to do with the loop app's.
2. Tag `follower-vX.Y.Z`. CI builds, signs and attaches **one APK per ABI**.
3. Add **two** `Builds:` entries with the new tag — same `versionName`, one per
   ABI, each with its own `versionCode` (`base × 10 + 1` for armv7, `+ 2` for
   arm64) and its own `output:` — and set `CurrentVersion` /
   `CurrentVersionCode` to the highest of them.

`UpdateCheckMode: Tags ^follower-v` means F-Droid notices new tags by itself.
`AutoUpdateMode` is deliberately `None`: it would write those entries for us
from `VercodeOperation`, but every copy would inherit one `output:` glob and so
both builds would pick the same ABI's APK. Step 3 is that, done honestly.

If the arithmetic in the recipe ever stops matching the arithmetic in
`build.gradle.kts`, F-Droid's build fails on the versionCode it did not expect
rather than publishing an APK that misdeclares its version. That is the only
check on it, so keep the two together.

## The signing key

F-Droid builds from source and signs with **their** key, so the key in this
project's CI is not what reaches F-Droid users. It matters for the GitHub
releases, for anyone self-hosting a repository, and for F-Droid's reproducible
builds path, where they verify their build matches a developer-signed APK and
publish that instead.

It lives outside every repository. **Losing it ends the update path for anyone
who installed a signed build from here.**
