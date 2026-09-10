//! Does p2panda's log sync carry an operation body, and how big a one?
//!
//! WHY THIS IS THE NEXT QUESTION. `crates/diaswarm-spaces` moved record
//! ciphertext out of the operation header — where `p2panda-core` caps decoding
//! at `.length_limit(512)` — and into the operation body. On this subject's
//! real history that turned 10,569 operations into 79, 3x wire inflation into
//! 1.01x, and twenty-seven minutes into five seconds.
//!
//! All of which is worth nothing if replication cannot move a body, or caps it
//! somewhere lower than the header. `p2panda-net`'s frame codec defaults to
//! 128 MB, which suggests there is room, but a default in a codec is not a
//! measurement of what a sync session actually transfers.
//!
//! So: one peer writes operations with bodies from 1 KB to 4 MB, another syncs
//! the topic they are associated with, and we compare what arrives against what
//! was written. **Byte-for-byte**, not just "an operation arrived" — a body
//! silently truncated to zero would otherwise look like a success.
//!
//! Run: cargo run --manifest-path spike/p2panda-logsync/Cargo.toml

use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;
use futures_util::StreamExt;
use p2panda_core::{Body, Hash, Header, Operation, SigningKey, Topic, VerifyingKey};
use p2panda_net::iroh_mdns::MdnsDiscoveryMode;
use p2panda_net::{AddressBook, Discovery, Endpoint, Gossip, LogSync, MdnsDiscovery};
use p2panda_sync::FromSync;
use p2panda_sync::protocols::TopicLogSyncEvent as Event;
use p2panda_store::logs::LogStore;
use p2panda_store::operations::OperationStore;
use p2panda_store::topics::TopicStore;
use p2panda_store::{SqliteStore, SqliteStoreBuilder, tx};

/// No extensions: this spike is about the body, and an extension type would
/// only add a second variable to a measurement with one question in it.
type Ext = ();
type LogId = u32;
type Store = SqliteStore;
type Sync = LogSync<Store, LogId, Ext>;

const LOG: LogId = 0;

struct Peer {
    _name: &'static str,
    key: SigningKey,
    store: Store,
    sync: Sync,
    _endpoint: Endpoint,
    _discovery: Discovery,
    _mdns: MdnsDiscovery,
}

impl Peer {
    async fn spawn(name: &'static str, network: [u8; 32]) -> Result<Self> {
        let store = SqliteStoreBuilder::new().database_url(":memory:").build().await?;
        let key = SigningKey::generate();

        let book = AddressBook::builder().spawn().await?;
        let endpoint = Endpoint::builder(book.clone())
            .signing_key(key.clone())
            .network_id(network)
            .spawn()
            .await?;
        // Active, or mDNS does nothing at all and the symptom is two peers that
        // never meet — the trap recorded in spike/p2panda-net.
        let mdns = MdnsDiscovery::builder(book.clone(), endpoint.clone())
            .mode(MdnsDiscoveryMode::Active)
            .spawn()
            .await?;
        let discovery = Discovery::builder(book.clone(), endpoint.clone()).spawn().await?;
        let gossip = Gossip::builder(book.clone(), endpoint.clone()).spawn().await?;
        let sync = Sync::builder(store.clone(), endpoint.clone(), gossip).spawn().await?;

        Ok(Peer { _name: name, key, store, sync, _endpoint: endpoint, _discovery: discovery, _mdns: mdns })
    }

    fn id(&self) -> VerifyingKey {
        self.key.verifying_key()
    }

    /// Append an operation carrying `payload` in its BODY.
    async fn write(&self, payload: Vec<u8>) -> Result<Hash> {
        let store = &self.store;
        let hash = tx!(store, {
            let (seq_num, backlink) =
                <Store as LogStore<Operation<Ext>, VerifyingKey, LogId, u32, Hash>>::get_latest_entry_tx(
                    store, &self.id(), &LOG,
                )
                .await?
                .map(|op| (op.header.seq_num + 1, Some(op.hash)))
                .unwrap_or((0, None));

            let header = Header::builder()
                .seq_num(seq_num)
                .backlink(backlink)
                .body(&payload)
                .build(&self.key, ());
            let op = Operation::from_parts(header, Some(Body::from_bytes(&payload)));
            store.insert_operation(&op.hash, &op, &LOG).await?;
            op.hash
        });
        Ok(hash)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Its own network id, so a test run never joins anything real — the hazard
    // measured in D19, where `cargo test` was found in the phones' pool.
    let network = *Hash::digest(b"diaswarm/spike/logsync").as_bytes();
    let topic: Topic = Hash::digest(b"diaswarm/spike/logsync/topic").into();

    let writer = Peer::spawn("writer", network).await?;
    let reader = Peer::spawn("reader", network).await?;

    println!("\n  p2panda-net 0.7.1 log sync — how much body crosses the wire?\n");

    // Both ends have to agree that this topic means "the writer's log".
    //
    // Inside a transaction: every write on `SqliteStore` runs against a permit
    // acquired with `begin`, and calling one without it fails with "tried to
    // interact with inexistant transaction" — which reads like store
    // corruption rather than a missing `tx!`.
    for peer in [&writer, &reader] {
        let store = &peer.store;
        tx!(store, { store.associate(&topic, &writer.id(), &LOG).await? });
    }

    // Bodies from small to absurd. The point is to find the ceiling, if there
    // is one, rather than to confirm that a comfortable size works.
    let sizes: Vec<usize> = vec![1 << 10, 1 << 14, 1 << 16, 1 << 18, 1 << 20, 1 << 22];
    let mut written: HashMap<Hash, usize> = HashMap::new();
    for size in &sizes {
        // Non-repeating, so a truncated or mis-assembled body cannot pass by
        // accidentally matching a length check.
        let payload: Vec<u8> = (0..*size).map(|i| (i % 251) as u8).collect();
        let hash = writer.write(payload).await?;
        written.insert(hash, *size);
    }
    println!("    writer wrote {} operations, {} KB total",
        written.len(),
        written.values().sum::<usize>() / 1024);

    let reader_handle = reader.sync.stream(topic, true).await?;
    let mut events = reader_handle.subscribe().await?;
    let _writer_handle = writer.sync.stream(topic, true).await?;

    // NO MANUAL KICK. `SyncHandle::initiate_session` exists and is `#[cfg(test)]`
    // — an application cannot start a session by hand, so sync has to come from
    // discovery finding a peer that shares the topic. That is the realistic
    // path anyway; it just means waiting for mDNS rather than dialling.
    println!("    waiting for discovery to bring them together…");

    let mut got: HashMap<Hash, usize> = HashMap::new();
    let mut mismatched = 0usize;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);

    while got.len() < written.len() && tokio::time::Instant::now() < deadline {
        let next = tokio::time::timeout_at(deadline, events.next()).await;
        let Ok(Some(Ok(FromSync { event, .. }))) = next else { break };
        match event {
            Event::OperationReceived { operation, .. } => {
                let body = operation.body.as_ref().map(|b| b.to_bytes()).unwrap_or_default();
                let expected = written.get(&operation.hash).copied();

                // BYTE FOR BYTE. A body that arrived truncated, or empty, would
                // otherwise be indistinguishable from one that arrived.
                let intact = body.iter().enumerate().all(|(i, b)| *b == (i % 251) as u8);
                if expected != Some(body.len()) || !intact {
                    mismatched += 1;
                    println!(
                        "    MISMATCH: expected {:?} bytes, got {} ({})",
                        expected,
                        body.len(),
                        if intact { "contents ok" } else { "CONTENTS WRONG" }
                    );
                }
                got.insert(operation.hash, body.len());
            }
            Event::SyncFinished { .. } => println!("    sync finished"),
            _ => {}
        }
    }

    println!();
    let mut sizes_got: Vec<usize> = got.values().copied().collect();
    sizes_got.sort_unstable();
    for size in &sizes {
        let arrived = got.values().any(|g| g == size);
        println!(
            "    {:>7} KB body   {}",
            size / 1024,
            if arrived { "arrived intact" } else { "DID NOT ARRIVE" }
        );
    }

    println!("\n  the answer\n");
    if got.len() == written.len() && mismatched == 0 {
        println!("    Log sync carries operation bodies, at every size tried up to");
        println!("    {} MB. Moving the payload out of the header costs nothing in",
            sizes.iter().max().unwrap() / (1 << 20));
        println!("    replication.");
    } else {
        println!("    {} of {} operations arrived, {mismatched} damaged.",
            got.len(), written.len());
        println!("    The largest body that made it was {} KB.",
            sizes_got.last().copied().unwrap_or(0) / 1024);
        println!("    That is a ceiling on how much a single operation can carry,");
        println!("    and diaswarm-spaces' MAX_PAYLOAD has to sit under it.");
    }

    Ok(())
}
