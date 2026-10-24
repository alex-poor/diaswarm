#!/usr/bin/env bash
#
# Assert the three products agree on the wire PROTOCOL_VERSION.
#
# WHY THIS EXISTS. The protocol — topic derivation, invite framing, log shape —
# is one number declared in `diaswarm-core`, but each of the three products
# carries its own copy of the expectation: the carrier reads the Rust const
# directly, while the two Android apps (Ayni, the AAPS plugin) hold a Kotlin
# `EXPECTED_PROTOCOL_VERSION` that their `check()` asserts against the native
# library at load. If someone bumps the Rust const for a protocol change and
# forgets one app's Kotlin — or the reverse — the fleet half-upgrades and the
# halves derive different topics and never meet (see the deploy-as-one-unit
# rule). The wire_contract test pins the Rust const's *bytes*; this pins that the
# Kotlin copies have not drifted away from it.
#
# Pure text extraction so it runs anywhere — CI, a release job, a laptop — with
# no toolchain. Prints the table it checked, exits non-zero on any mismatch.
set -euo pipefail

cd "$(dirname "$0")/.."

CORE=crates/diaswarm-core/src/lib.rs
FOLLOWER=follower/src/main/kotlin/nz/diaswarm/jni/SwarmNative.kt
PLUGIN=plugin/src/main/kotlin/nz/diaswarm/jni/SwarmNative.kt

# Pull the single integer out of a `... = N` line; fail loudly if the pattern
# moved, rather than silently extracting nothing and comparing empties.
num() {
  local file="$1" pattern="$2" hit
  hit=$(grep -oE "$pattern" "$file" | grep -oE '[0-9]+$' | head -1 || true)
  if [ -z "$hit" ]; then
    echo "protocol-version-check: could not find '$pattern' in $file" >&2
    exit 2
  fi
  printf '%s' "$hit"
}

core=$(num "$CORE" 'PROTOCOL_VERSION: u32 = [0-9]+')
follower=$(num "$FOLLOWER" 'EXPECTED_PROTOCOL_VERSION = [0-9]+')
plugin=$(num "$PLUGIN" 'EXPECTED_PROTOCOL_VERSION = [0-9]+')

printf 'PROTOCOL_VERSION\n'
printf '  carrier / core (diaswarm-core)   %s\n' "$core"
printf '  Ayni follower (Kotlin)           %s\n' "$follower"
printf '  AAPS plugin (Kotlin)             %s\n' "$plugin"

if [ "$core" = "$follower" ] && [ "$core" = "$plugin" ]; then
  printf 'OK: all three products speak protocol v%s\n' "$core"
  exit 0
fi

echo "MISMATCH: the three products disagree on PROTOCOL_VERSION (see above)." >&2
echo "Bump all three together, update crates/diaswarm-net/tests/wire_contract.rs," >&2
echo "and ship the fleet as one unit." >&2
exit 1
