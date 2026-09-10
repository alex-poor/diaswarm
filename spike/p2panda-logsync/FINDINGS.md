# p2panda-net 0.7.1 log sync, measured

**Run:** `cargo run --manifest-path spike/p2panda-logsync/Cargo.toml`

The question left open by `../p2panda-spaces`: `crates/diaswarm-spaces` moved
record ciphertext out of the operation header — where `p2panda-core` caps
decoding at `.length_limit(512)` — and into the operation **body**, turning
10,569 operations into 79 and twenty-seven minutes into five seconds on real
history. None of which is worth anything if replication cannot move a body.

---

## 1. Bodies replicate, at every size tried

```
      1 KB body   arrived intact
     16 KB body   arrived intact
     64 KB body   arrived intact
    256 KB body   arrived intact
   1024 KB body   arrived intact
   4096 KB body   arrived intact
```

Compared **byte for byte**, not by length: the payload is a non-repeating
pattern, so a truncated or mis-assembled body cannot pass by accident. A body
that arrived empty would otherwise be indistinguishable from one that arrived.

`p2panda-net`'s frame codec defaults to a 128 MB limit and is configurable, so
4 MB is nowhere near it. **`diaswarm-spaces`' 64 KB `MAX_PAYLOAD` has two orders
of magnitude of headroom.**

## 2. Sync is started by discovery, not by the application

`SyncHandle::initiate_session` exists and is `#[cfg(test)]`, with an upstream
TODO wondering whether to make it public. An application cannot start a sync
session by hand: it associates a topic with an author's logs, subscribes, and
waits for discovery to find a peer that shares the topic.

That is the realistic path and it is what diaswarm already has — the pool's
bucket topics are exactly this mapping. It does mean **there is no way to say
"sync with this peer now"**, which matters for a follower that has just scanned
an invite and would like its data immediately rather than whenever discovery
gets round to it.

## 3. The shape a migration would take

`LogSync<S, L, E>` needs a store implementing `LogStore` and `TopicStore` —
`SqliteStore` does both — plus an endpoint and gossip, which the pool already
has. Then `associate(topic, author, log_id)` and `stream(topic, live_mode)`.
Live mode pushes new operations over gossip after the initial catch-up, which
would remove the two-minute follower poll that
[D17](../../docs/decisions.md) settled and §12.3 complains about.

This replaces `crates/diaswarm-net/src/wire.rs` — `Have`, `Manifest`, `Grants`,
`Segment`, `Wraps` — entirely.

## 4. A trap worth writing down

Every write on `SqliteStore` runs against a permit taken with `begin`, including
the methods that do not end in `_tx` and look self-contained. Calling
`associate` outside one fails with **"tried to interact with inexistant
transaction"**, which reads like store corruption rather than a missing `tx!`.
