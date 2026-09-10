//! How long after scanning a code does a follower actually see anything?
//!
//! THE ONE NUMBER A DESIGN DECISION IS WAITING ON. Today a scan triggers a
//! direct fetch: parse the invite, dial the address in it, pull the data. Under
//! log sync there is no such thing — `SyncHandle::initiate_session` is
//! `#[cfg(test)]` upstream — so a follower associates a topic, subscribes, and
//! waits for **discovery** to introduce the two peers. Nothing lets an
//! application say "I know who I want, go and get it".
//!
//! That is a worse first impression, and the question is how much worse. Five
//! seconds is a progress spinner. Five minutes is a person scanning the code
//! again because it looks broken.
//!
//! Both paths are measured here, cold, several times:
//!
//!   * **direct dial** — what the invite does today: connect to a known address
//!     and open a stream. No discovery involved.
//!   * **log sync** — subscribe to a topic and wait for the first operation to
//!     arrive from a peer nobody was told about.
//!
//! Each iteration uses a fresh network id so no discovery state carries over
//! from the last one; otherwise the second measurement is of a warm cache and
//! flatters the answer.
//!
//! Run: cargo run --manifest-path spike/p2panda-logsync/Cargo.toml --bin firstcontact

use std::time::{Duration, Instant};

use anyhow::Result;
use futures_util::StreamExt;
use p2panda_core::{Body, Hash, Header, Operation, SigningKey, Topic, VerifyingKey};
use p2panda_net::iroh_mdns::MdnsDiscoveryMode;
use p2panda_net::{AddressBook, Discovery, Endpoint, Gossip, LogSync, MdnsDiscovery};
use p2panda_store::logs::LogStore;
use p2panda_store::operations::OperationStore;
use p2panda_store::topics::TopicStore;
use p2panda_store::{SqliteStore, SqliteStoreBuilder, tx};
use p2panda_sync::FromSync;
use p2panda_sync::protocols::TopicLogSyncEvent as Event;

type Ext = ();
type LogId = u32;
type Sync = LogSync<SqliteStore, LogId, Ext>;
const LOG: LogId = 0;

struct Peer {
    key: SigningKey,
    store: SqliteStore,
    sync: Sync,
    endpoint: Endpoint,
    _discovery: Discovery,
    _mdns: MdnsDiscovery,
}

async fn spawn(network: [u8; 32]) -> Result<Peer> {
    let store = SqliteStoreBuilder::new().database_url(":memory:").build().await?;
    let key = SigningKey::generate();
    let book = AddressBook::builder().spawn().await?;
    let endpoint = Endpoint::builder(book.clone())
        .signing_key(key.clone())
        .network_id(network)
        .spawn()
        .await?;
    let mdns = MdnsDiscovery::builder(book.clone(), endpoint.clone())
        .mode(MdnsDiscoveryMode::Active)
        .spawn()
        .await?;
    let discovery = Discovery::builder(book.clone(), endpoint.clone()).spawn().await?;
    let gossip = Gossip::builder(book.clone(), endpoint.clone()).spawn().await?;
    let sync = Sync::builder(store.clone(), endpoint.clone(), gossip).spawn().await?;
    Ok(Peer { key, store, sync, endpoint, _discovery: discovery, _mdns: mdns })
}

impl Peer {
    fn id(&self) -> VerifyingKey {
        self.key.verifying_key()
    }

    async fn write(&self, payload: Vec<u8>) -> Result<()> {
        let store = &self.store;
        tx!(store, {
            let (seq_num, backlink) =
                <SqliteStore as LogStore<Operation<Ext>, VerifyingKey, LogId, u32, Hash>>::get_latest_entry_tx(
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
        });
        Ok(())
    }
}

/// Subscribe to a topic and wait for the first operation to arrive.
async fn time_log_sync(run: usize) -> Result<Option<Duration>> {
    // A fresh network per run: discovery state from the last one would make
    // this a measurement of a warm cache.
    let network = *Hash::digest(format!("diaswarm/firstcontact/{run}").as_bytes()).as_bytes();
    let topic: Topic = Hash::digest(format!("diaswarm/firstcontact/topic/{run}").as_bytes()).into();

    let writer = spawn(network).await?;
    // A day of records, so this is a realistic first fetch rather than a ping.
    for i in 0..40 {
        writer.write(vec![(i % 251) as u8; 1024]).await?;
    }
    let reader = spawn(network).await?;

    for peer in [&writer, &reader] {
        let store = &peer.store;
        tx!(store, { store.associate(&topic, &writer.id(), &LOG).await? });
    }

    // The clock starts where a person's patience does: the moment the follower
    // has been told who to follow.
    let started = Instant::now();
    let reader_handle = reader.sync.stream(topic, true).await?;
    let mut events = reader_handle.subscribe().await?;
    let _writer_handle = writer.sync.stream(topic, true).await?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    while tokio::time::Instant::now() < deadline {
        let Ok(Some(Ok(FromSync { event, .. }))) = tokio::time::timeout_at(deadline, events.next()).await
        else {
            break;
        };
        if let Event::OperationReceived { .. } = event {
            return Ok(Some(started.elapsed()));
        }
    }
    Ok(None)
}

/// Dial a known address and open a stream — what an invite does today.
async fn time_direct_dial(run: usize) -> Result<Duration> {
    let network = *Hash::digest(format!("diaswarm/direct/{run}").as_bytes()).as_bytes();
    let server = spawn(network).await?;
    let client = spawn(network).await?;

    let addr = server.endpoint.endpoint().await?.addr();
    let alpn = b"diaswarm/firstcontact";
    server
        .endpoint
        .accept(alpn, EchoProtocol)
        .await
        .map_err(|e| anyhow::anyhow!("accept: {e}"))?;

    // The clock starts at the same place: the follower knows who to talk to.
    let started = Instant::now();
    let ep = client.endpoint.endpoint().await?;
    let conn = ep.connect(addr, &mixed(alpn, network)[..]).await?;
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(b"hello").await?;
    send.finish()?;
    let _ = recv.read_to_end(1024).await?;
    Ok(started.elapsed())
}

/// p2panda mixes the protocol id with its network id before iroh sees it.
fn mixed(alpn: &[u8], network: [u8; 32]) -> Vec<u8> {
    Hash::digest([alpn, &network[..]].concat()).as_bytes().to_vec()
}

#[derive(Debug, Clone)]
struct EchoProtocol;

impl iroh::protocol::ProtocolHandler for EchoProtocol {
    async fn accept(&self, connection: iroh::endpoint::Connection) -> Result<(), iroh::protocol::AcceptError> {
        while let Ok((mut send, mut recv)) = connection.accept_bi().await {
            let got = recv.read_to_end(1024).await.unwrap_or_default();
            let _ = send.write_all(&got).await;
            let _ = send.finish();
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    const RUNS: usize = 5;
    println!("\n  first contact: how long until a follower sees anything?\n");

    let mut direct = Vec::new();
    for run in 0..RUNS {
        let d = time_direct_dial(run).await?;
        println!("    direct dial   run {run}: {:>8.0?}", d);
        direct.push(d);
    }

    println!();
    let mut synced = Vec::new();
    for run in 0..RUNS {
        match time_log_sync(run).await? {
            Some(d) => {
                println!("    log sync      run {run}: {:>8.0?}", d);
                synced.push(d);
            }
            None => println!("    log sync      run {run}: nothing within 120s"),
        }
    }

    let median = |mut v: Vec<Duration>| -> Duration {
        v.sort();
        v.get(v.len() / 2).copied().unwrap_or_default()
    };

    println!("\n  median\n");
    println!("    direct dial (today)   {:.0?}", median(direct));
    if synced.is_empty() {
        println!("    log sync              never arrived");
    } else {
        println!(
            "    log sync (proposed)   {:.0?}   {} of {RUNS} arrived",
            median(synced.clone()),
            synced.len()
        );
    }
    Ok(())
}
