#!/usr/bin/env bash
# Build (and optionally install) AAPS with the swarm plugin.
#
# A thin wrapper over camaps' build-loop-apk.sh, which owns the parts that
# matter: it refuses to install if the signing certificate no longer matches
# the device (that check is what protects the pump key), saves a rollback APK,
# and relaunches + AOT-compiles afterwards.
#
# All three variables below are needed and none of them are discoverable:
#
#   REPO    the git WORKTREE holding the diaswarm-addon branch. The AAPS
#           checkout itself must stay on main and clean — a previous session
#           left uncommitted work on it because another one ran `git checkout`.
#   BRANCH  matches that worktree, so the script's on-the-right-branch guard
#           still means something instead of being waved through.
#   ANDROID_NDK_HOME
#           the plugin builds the Rust core into a .so; without it the build
#           fails late, having already spent a couple of minutes.
#
#   ./plugin/build-apk.sh              build only
#   ./plugin/build-apk.sh --install    build, verify signer, install, relaunch
#
# Do NOT wrap this in `timeout`. A killed KSP build leaves a worse state than
# no build; camaps' script says the same thing and it has been ignored before.
set -euo pipefail

CAMAPS=/home/alex/projects/camaps
export REPO="${REPO:-$CAMAPS/sdk/aaps-diaswarm}"
export BRANCH="${BRANCH:-diaswarm-addon}"
export SUFFIX="${SUFFIX:-swarm}"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$(ls -1d "$CAMAPS"/tools/android-sdk/ndk/*/ | sort -V | tail -1)}"
export PATH="$PATH:$CAMAPS/tools/platform-tools"

[ -d "$REPO" ] || { echo "!! No worktree at $REPO" >&2; exit 1; }
exec bash "$CAMAPS/sdk/build-loop-apk.sh" "$@"
