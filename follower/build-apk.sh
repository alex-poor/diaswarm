#!/usr/bin/env bash
# Build Ayni (the follower) and sign it with the release key.
#
# WHY THIS EXISTS. The recipe — toolchain paths, cargo-ndk, per-ABI splits,
# zipalign, apksigner, the keystore — was re-derived by hand every time and got
# it wrong at least twice, once shipping an APK whose native half was a build
# behind the Kotlin. The plugin has `plugin/build-apk.sh` for the same reason.
#
#   ./follower/build-apk.sh [--install]
#
# --install pushes the arm64 APK to $ANDROID_SERIAL (phone B, never the loop
# phone, which does not run Ayni).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tools="$HOME/projects/camaps/tools"

# THE MACHINE DEFAULTS FAIL HERE with a misleading jlink error; see
# `docs/` and the aaps-build-toolchain note. These are the ones that work.
# `ls -1d`, not `echo` with a glob: `echo dir/*/ | head -1` is one line holding
# EVERY match, so `head` takes all of them and the NDK path becomes two paths
# joined by a space. cargo-ndk then reports "No such file or directory" about a
# path that plainly exists, which is a confusing way to spend ten minutes.
export JAVA_HOME="${JAVA_HOME:-$(ls -1d "$tools"/jdk*/ | sort -V | tail -1)}"
export ANDROID_HOME="${ANDROID_HOME:-$tools/android-sdk}"
export ANDROID_SDK_ROOT="$ANDROID_HOME"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$(ls -1d "$ANDROID_HOME"/ndk/*/ | sort -V | tail -1)}"
export PATH="$JAVA_HOME/bin:$PATH"

echo "== [1/3] building =="
echo "   JAVA_HOME=$JAVA_HOME"
echo "   NDK=$ANDROID_NDK_HOME"
(cd "$here" && ./gradlew --no-daemon assembleRelease)

buildtools="$(ls -d "$ANDROID_HOME"/build-tools/*/ | sort -V | tail -1)"
keystore="$HOME/.diaswarm-signing/follower.jks"
password="$(cat "$HOME/.diaswarm-signing/password")"
out="$here/build/outputs/apk/release"

echo "== [2/3] aligning and signing =="
signed=()
for unsigned in "$out"/*-release-unsigned.apk; do
    [ -e "$unsigned" ] || { echo "no unsigned APK in $out" >&2; exit 1; }
    base="$(basename "$unsigned" -unsigned.apk)"
    aligned="$out/$base-aligned.apk"
    final="$out/$base.apk"
    "$buildtools/zipalign" -f -p 4 "$unsigned" "$aligned"
    "$buildtools/apksigner" sign \
        --ks "$keystore" --ks-pass "pass:$password" --key-pass "pass:$password" \
        --out "$final" "$aligned"
    rm -f "$aligned" "$final.idsig"
    signed+=("$final")
done

echo "== [3/3] built =="
for f in "${signed[@]}"; do
    printf '   %-52s %s\n' "$(basename "$f")" "$(du -h "$f" | cut -f1)"
done

if [ "${1:-}" = "--install" ]; then
    arm64="$(printf '%s\n' "${signed[@]}" | grep arm64 | head -1)"
    : "${ANDROID_SERIAL:?set ANDROID_SERIAL — phone B, not the loop phone}"
    echo "== installing $(basename "$arm64") on $ANDROID_SERIAL =="
    adb -s "$ANDROID_SERIAL" install -r "$arm64"
    adb -s "$ANDROID_SERIAL" shell monkey -p nz.diaswarm.ayni -c android.intent.category.LAUNCHER 1 >/dev/null
    echo "   started; pid $(adb -s "$ANDROID_SERIAL" shell pidof nz.diaswarm.ayni || echo 'NOT RUNNING')"
fi
