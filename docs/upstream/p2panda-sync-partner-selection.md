# Draft: sync partner selection depends on HyParView sampling

**For p2panda-net / p2panda-sync 0.7.1. Not filed — draft for review.**

## What we observe

A peer that carries a subject catches up once on connect and then receives
nothing further, indefinitely, while reporting healthy totals. Every sync
session logs `LiveModeStarted` and `received_live_operations: 0`.

It is not a stall in the transport. The peer has an open session with *one*
peer, which has nothing to send, and never opens one with the peer that has the
data.

## Why, reading 0.7.1

**1. A session is created only from HyParView membership events.**
`p2panda-net/src/sync/actors/manager.rs::spawn_membership_task`:

```rust
GossipEvent::Joined      { topic, ref nodes } => (topic, …, true)   // InitiateSync
GossipEvent::NeighbourUp { node, topic }      => (topic, vec![node], true)
GossipEvent::NeighbourDown { node, topic }    => (topic, vec![node], false) // EndSync
```

So the set of peers a node exchanges data with **is its active view**, per
topic. There is no other path. If sampling does not pair you with a peer holding
the data, nothing later corrects it: `NeighbourUp` only fires on a view change.

**2. Relay between sessions is same-topic only.**
`p2panda-sync/src/manager/event_stream.rs`:

```rust
let topic = state.session_topic_map.topic(session_id);
let keys  = state.session_topic_map.sessions(topic);   // same topic only
```

The protocol doc says live messages are *"forwarded to any concurrently running
sync sessions"*, which reads as unconditional. Two peers can be connected and
never relay if their sessions are on different topics.

**3. Each sync topic gets its own gossip overlay**, via
`derive_topic(topic, GOSSIP_TOPIC_MIX_VALUE)`. So an application using many
fine-grained topics gets many independent, sparse overlays — worse sampling
*and* worse relay. This is the opposite of the intuition that finer topics are
tidier, and it is not documented.

## Why an application cannot work around it

`SyncHandle::initiate_session(node_id)` is exactly the missing primitive — and
it is `#[cfg(test)]`, behind:

```rust
// TODO: Consider making this public, for this we would need to decide if we want to receive
// the sync session events and status directly as a stream from the return type?
```

The address book already records which nodes are interested in which topics
(`topics2node_infos_v1`), so the information needed to pick a partner is present
and unused.

## Suggestions, smallest first

1. **Make `initiate_session` public.** Fire-and-forget matches the existing
   design — events already reach the application through `subscribe()`, so the
   return-type question in the TODO need not block it.
2. **Seed initiation from the address book**, bounded and rotating, rather than
   only from active-view events. Uses data the library already keeps and fixes
   this without application changes.
3. **Document that relay is same-topic**, and that topic granularity affects
   connectivity.

## Our use case

A volunteer "carrier" holds sealed records for subjects it cannot read, so that
somebody's phone can be asleep and their data still reachable. It knows exactly
which subjects it wants. It does not need peer *discovery* — it needs to sync
with a peer that has a specific log, and today it can only hope sampling
provides one.

## What we did instead

Re-subscribed the topics after three passes with no new data, which re-rolls the
active view. It works and is plainly a workaround: data arrives in bursts rather
than continuously.
