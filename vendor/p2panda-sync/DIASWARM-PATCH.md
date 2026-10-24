# diaswarm patch of p2panda-sync 0.7.1

This is a vendored fork of the crates.io `p2panda-sync` 0.7.1, pulled in via
`[patch.crates-io]` from `crates/diaswarm-{net,peer,android}/Cargo.toml`.

## Why

`TopicSyncManager` (`src/manager/mod.rs`) held two per-session maps,
`from_session_tx` and `from_session_rx`, keyed by `(session_id, remote)`, that
were **insert-only** — nothing ever removed an entry. A sync session is started
on every gossip `NeighbourUp` (p2panda-net `sync/actors/manager.rs`), and
iroh-gossip's HyParView shuffles neighbours continuously, so a long-running peer
on a real overlay leaked one `broadcast::channel(1028)` per membership event, for
ever. The `from_session_tx` sender clone kept each ended session's channel (and
up to a ring of operations) alive, which also stopped that session's
`BroadcastStream` in the event stream's `SelectAll` from ever ending.

Measured on a diaswarm carrier: RSS climbed ~100 MB/min and never plateaued
(tens of GB of swap within an hour) while the stored op set was constant. Not
present in the toy/single-topic cases p2panda's tests cover, which is why it was
never noticed upstream. PR #1307 (graceful session close) cleaned the p2panda-net
actor layer, not this map; issue #1041 names the same mechanism as a data-loss
race, not a leak. Still present on `main`.

## The change (two files)

`src/manager/mod.rs`:
- delete `from_session_rx` entirely — a never-polled receiver per session, pure
  dead weight;
- make `from_session_tx` an `Arc<Mutex<HashMap<…>>>` (`FromSessionTx<E>`), shared
  with the event stream so it can be reaped; keep it (a late / re-`subscribe()`
  still needs to wire into a running session — diaswarm's `restream` stall
  recovery relies on this; removing it outright breaks delivery);
- prune dead subscribers from `manager_tx` in `session()` (also insert-only).

`src/manager/event_stream.rs`:
- add `FromSessionTx<E>` type alias and the field on `ManagerEventStreamState`;
- add `ManagerEventStreamState::drop_session(id)` that reaps both
  `session_topic_map` and `from_session_tx`, and route the three existing
  dead-session drop points through it.

## Result

Carrier RSS, same subject, before/after (sampled every 20 s): stock climbed
240→758 MB in 220 s and kept going; patched rose once to a ~619 MB catch-up
working set then stayed flat (±0.2 MB over 5 min). All 26 p2panda-sync unit tests
pass. No wire/format change.

## Removing this fork

Drop the `p2panda-sync = { path = "../../vendor/p2panda-sync" }` lines from the
three `Cargo.toml`s and delete this directory once the fix is released upstream.
See memory note `diaswarm-ayni-leak-is-reader-readcost` and the upstream issue.
