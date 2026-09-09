# Does p2panda-net do what `diaswarm-net` was reinventing?

Yes, and the answer took about forty lines.

D2a named the trigger — *"the moment two devices must sync"* — and said
`p2panda-net` over iroh was the answer. That moment arrived and was met with a
bespoke transport (D14), then bespoke holder discovery (D18), then the start of
a bespoke DHT. This spike is the check that should have happened first.

## What was measured, on 0.7.1

| Question | Answer |
|---|---|
| Builds on the host? | Yes — but needs **rustc 1.96+** |
| Cross-compiles for `aarch64-linux-android`? | **Yes.** The risk that would have ended this |
| Do two nodes find each other with nobody told? | **Yes, under 10s on a LAN** |
| Does gossip carry a message between them? | Yes |
| Is the underlying iroh endpoint reachable? | **Yes — `Endpoint::endpoint()`** |

That last one decides the shape of the migration. Our `VaultServer` speaks its
own ALPN; if it can be attached to the endpoint p2panda is already using, then
discovery and gossip can be adopted **without** rewriting how segments move.
A staged port, not a rewrite.

## Run it

```sh
cargo run -- A     # one terminal
cargo run -- B     # another
```

```
[A] GOSSIP HEARD: hello from B
[A] t+ 10s  nodes on topic: 2
```

## The trap

`MdnsDiscovery` spawned without `.mode(MdnsDiscoveryMode::Active)` does
nothing whatsoever, and the symptom is two nodes sitting at "nodes on topic: 1"
for as long as you care to watch — indistinguishable from discovery being
broken, or from the two processes being unable to reach each other.

## What this does NOT settle

p2panda-net is **topic-based**: peers subscribe to a topic and sync it. It
supplies discovery, gossip, an address book and log sync. It does **not**
supply shard assignment — "everyone holds *chunks* of everyone's data". That
still needs a rule on top (rendezvous hashing over peer id and chunk id), which
is arithmetic and independent of transport.

Holochain has sharding natively, which is why it keeps coming up. D2 rejected
it for having no key layer for public entries — that reasoning is untouched by
this spike.

## Version note

Pinned to `=0.7.1`, and `rust-toolchain.toml` pins 1.98.1: the crates the
plugin ships are built with an older default, and this spike must not drag them
forward as a side effect.
