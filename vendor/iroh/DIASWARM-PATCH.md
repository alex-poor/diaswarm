# iroh 1.1.0, vendored and patched

Upstream: <https://github.com/n0-computer/iroh> — MIT OR Apache-2.0, unchanged.
This is the published `iroh` 1.1.0 crate with **one** change.

## Why

`RemoteStateActor` grows `State::pending_open_paths` without bound. When
`open_path_ensure` fails with `MaxPathIdReached` or `RemoteCidsExhausted`,
`open_path_on_conn` pushes the address onto the queue and schedules a 333 ms
retry. The retry drains the queue and calls `open_path_on_all_conns` for each
entry — which re-attempts the address on *every* connection to that peer, and
every connection still at the path-id cap pushes it back again.

So one entry becomes C entries per tick, C being the connections at the cap, and
the `VecDeque` doubles its way to multiple gigabytes. With a single connection it
is steady state, which is why it hides.

**Upstream issues, both open at the time of writing:**

* [#4390](https://github.com/n0-computer/iroh/issues/4390) — `bug`, milestone
  Sprint P. 24 GB single allocation on macOS. Carries the hot-fix used here.
* [#4509](https://github.com/n0-computer/iroh/issues/4509) — Windows, RSS past
  100 GB, `ALLOCATION FAILURE: 171798691840 bytes requested`.
* [#4124](https://github.com/n0-computer/iroh/issues/4124) — closed, the
  related `open_path` warning spam.

**Measured here on Android, 2026-09-15/16**, with heapprofd on a follower that
had been up 54 minutes:

```
largest SINGLE allocation   834,695,168 bytes  (796 MB)
total allocated on stack      1.13 GB in 150 s
```

`iroh 1.2.0` carries the identical logic — only a variable is renamed — so
upgrading does not help. Checked, not assumed.

## The change

One function and one call site in `src/socket/remote_map/remote_state.rs`:
`enqueue_pending_open_path` dedupes and caps the queue at 64. It is verbatim the
hot-fix posted by cbenhagen in #4390, so it should disappear cleanly when the
real one lands.

## Removing this

When upstream fixes it: delete `vendor/iroh`, drop the `[patch.crates-io]`
stanzas from `crates/diaswarm-net/Cargo.toml` and
`crates/diaswarm-android/Cargo.toml`, and bump the `iroh` requirement to the
first released version carrying the fix.

**`crates/diaswarm-peer` is deliberately NOT patched.** It runs stock iroh 1.2.0
on a desktop, so it is the control: if the phones stop growing and the peer does
not, that is the patch working rather than the weather.
