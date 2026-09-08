#!/usr/bin/env bash
# Exercise the real native library across a real JNI boundary.
#
# WHY THIS EXISTS. `cargo test` proves the Rust is right and the Gradle build
# proves the Kotlin compiles, and neither proves the two ever meet. A mangled
# symbol that does not match, a handle that is not a pointer, an exception
# thrown across the boundary — all of that is invisible until the library loads
# on a phone. This loads it on the host instead.
#
# It mirrors SwarmNative.kt's declarations in Java rather than compiling the
# Kotlin, because the JNI short name does not encode static-ness: the symbols
# resolve identically, and this needs no Kotlin toolchain.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE="$(dirname "$HERE")"

JAVA_HOME="${JAVA_HOME:-$(ls -d "$HOME"/.local/jdk/jdk-21* 2>/dev/null | head -1)}"
[[ -x "$JAVA_HOME/bin/javac" ]] || { echo "set JAVA_HOME to a JDK" >&2; exit 1; }

cargo build --release --manifest-path "$CRATE/Cargo.toml"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
"$JAVA_HOME/bin/javac" -d "$WORK" \
    "$HERE/app/aaps/plugins/sync/swarm/SwarmNative.java" "$HERE/Check.java"
"$JAVA_HOME/bin/java" -cp "$WORK" \
    -Djava.library.path="$CRATE/target/release" Check
