//! Does a peer end up holding a stranger's data, knowing only a topic?
//!
//! The swarm property, on the stack that would replace `wire.rs`. Nobody is
//! dialled and nothing is requested: one peer seals a subject's history, the
//! other associates that subject's log with the bucket topic it carries, and
//! p2panda's log sync does the rest.
//!
//! `tests/swarm.rs` proves the same property over the hand-written protocol.
//! This is the same claim, made of the library instead.

use std::time::Duration;

use diaswarm_core::{EPOCH_MS, Record};
use diaswarm_net::pool;
use diaswarm_net::replicate::Replicator;
use diaswarm_net::swarm::{network_id, Swarm};
use diaswarm_spaces::{Reach, Vault};
use p2panda_core::{Hash, Operation, SigningKey, VerifyingKey};
use p2panda_store::logs::LogStore;
use p2panda_store::SqliteStore;

const OFFSET: i64 = 12 * 3_600_000;

fn tmp(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-repl-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn day(epoch: i64, mgdl: f64) -> Vec<Record> {
    vec![Record::new(epoch * EPOCH_MS + 3_600_000, "cgm").set("mgdl", Some(mgdl.into()))]
}

/// Every operation of this author's log a store holds, in log order.
///
/// Read back out of the CARRIER's store, so a relay test relays rather than
/// being handed the subject's own messages.
async fn operations_of(store: &SqliteStore, author: &VerifyingKey) -> Vec<diaswarm_spaces::Operation> {
    let entries = <SqliteStore as LogStore<
        Operation<diaswarm_spaces::SpacesArgs<()>>,
        VerifyingKey,
        u32,
        u32,
        Hash,
    >>::get_log_entries(store, author, &diaswarm_spaces::LOG_ID, None, None)
    .await
    .unwrap();
    // `LogEntries` is (operation, body-bytes) pairs; the operation already
    // carries its body, so the second half is not needed here.
    entries
        .map(|e| e.into_iter().map(|(op, _)| diaswarm_spaces::Operation::wrap(op)).collect())
        .unwrap_or_default()
}

/// How many operations of this author's log a store holds.
async fn held(store: &SqliteStore, author: &VerifyingKey) -> u32 {
    let size = <SqliteStore as LogStore<
        Operation<diaswarm_spaces::SpacesArgs<()>>,
        VerifyingKey,
        u32,
        u32,
        Hash,
    >>::get_log_size(store, author, &diaswarm_spaces::LOG_ID, None, None)
    .await
    .unwrap();
    // (OPERATIONS, BYTES) — in that order, whatever the trait's doc comment
    // says. Taking the second element and calling it a count compares bytes
    // against a number of operations: always far larger, so every assertion
    // built on it passed without testing anything.
    size.map(|(ops, _bytes)| ops).unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_peer_carries_a_subject_it_was_never_introduced_to() {
    let net = network_id("replicate-carry");

    // --- the subject: seals a few days, and is in the pool ----------------
    let mut vault = Vault::open(tmp("subject"), OFFSET).await.unwrap();
    let subject = vault.subject().to_hex();
    let mut sealed = 0usize;
    for e in 0..3i64 {
        sealed += vault.seal(&day(22_000 + e, 100.0 + e as f64)).await.unwrap().len();
    }
    assert!(sealed > 0);

    let publisher = Swarm::join_network(tmp("pub-pool"), SigningKey::generate(), net)
        .await
        .unwrap();
    let (pub_endpoint, pub_gossip) = publisher.parts();
    let publishing = Replicator::start(vault.store(), pub_endpoint, pub_gossip).await.unwrap();

    // --- the carrier: a stranger, granted nothing, holding a bucket -------
    let carrier_vault = Vault::open(tmp("carrier"), OFFSET).await.unwrap();
    let carrier = Swarm::join_network(tmp("car-pool"), SigningKey::generate(), net).await.unwrap();
    let (car_endpoint, car_gossip) = carrier.parts();
    let carrying = Replicator::start(carrier_vault.store(), car_endpoint, car_gossip)
        .await
        .unwrap();

    // The one thing both know: which bucket this subject falls in. Neither was
    // told the other exists.
    let depth = pool::depth_for(2);
    let topic = pool::bucket_topic(depth, pool::bucket_of(&subject, depth));
    publishing.carry(topic, &subject).await.unwrap();
    carrying.carry(topic, &subject).await.unwrap();

    let author = vault.subject();
    let mut carried: u32 = 0;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        carried = held(&carrier_vault.store(), &author).await;
        if carried as usize >= sealed {
            break;
        }
    }

    println!("  publisher events: {:?}", publishing.events());
    println!("  carrier events:   {:?}", carrying.events());
    assert!(
        carried as usize >= sealed,
        "the carrier holds {carried} of the subject's {sealed} operations — \
         replication did not happen"
    );
    assert!(carrying.received() > 0, "no operations were reported as received");
}

/// And holding is not reading: the carrier cannot open a byte of it.
#[tokio::test(flavor = "multi_thread")]
async fn what_it_carries_it_cannot_read() {
    let net = network_id("replicate-opaque");

    let mut vault = Vault::open(tmp("o-subject"), OFFSET).await.unwrap();
    let reader = Vault::open(tmp("o-reader"), OFFSET).await.unwrap();
    vault.register(&reader).await.unwrap();
    reader.register(&vault).await.unwrap();

    let subject = vault.subject().to_hex();
    let mut ops = vault.seal(&[]).await.unwrap();
    ops.extend(vault.grant(reader.subject(), Reach::Everything).await.unwrap());
    ops.extend(vault.seal(&day(22_100, 117.0)).await.unwrap());

    let publisher = Swarm::join_network(tmp("o-pub"), SigningKey::generate(), net).await.unwrap();
    let (e, g) = publisher.parts();
    let publishing = Replicator::start(vault.store(), e, g).await.unwrap();

    let carrier_vault = Vault::open(tmp("o-carrier"), OFFSET).await.unwrap();
    let carrier = Swarm::join_network(tmp("o-car"), SigningKey::generate(), net).await.unwrap();
    let (e, g) = carrier.parts();
    let carrying = Replicator::start(carrier_vault.store(), e, g).await.unwrap();

    let depth = pool::depth_for(2);
    let topic = pool::bucket_topic(depth, pool::bucket_of(&subject, depth));
    publishing.carry(topic, &subject).await.unwrap();
    carrying.carry(topic, &subject).await.unwrap();

    let author = vault.subject();
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if held(&carrier_vault.store(), &author).await as usize >= ops.len() {
            break;
        }
    }
    assert!(
        held(&carrier_vault.store(), &author).await > 0,
        "nothing replicated, so there is nothing to fail to read"
    );

    // The granted reader opens it; the carrier does not.
    assert!(
        !reader.ingest(&ops).await.unwrap().records.is_empty(),
        "the granted reader read nothing"
    );
    assert!(
        carrier_vault.ingest(&ops).await.unwrap().records.is_empty(),
        "the carrier opened data it was never granted"
    );
}

/// THE WHOLE CHAIN, END TO END, ON THE NEW STACK.
///
/// Everything else tests a link: the carrier holds ciphertext, the reader can
/// decrypt, log sync moves bodies. This is the claim the project actually
/// makes, which is that those links compose — a subject seals, a stranger
/// carries it without being asked, and a granted reader gets the data **from
/// the stranger**, opening what the subject sealed.
///
/// It is the last thing in `docs/migration.md`'s list that had never been
/// tested on the p2panda stack.
#[tokio::test(flavor = "multi_thread")]
async fn a_granted_reader_gets_a_subject_from_a_peer_that_is_not_the_subject() {
    let net = network_id("replicate-relay");

    // --- the subject grants a reader, then seals ---------------------------
    let mut vault = Vault::open(tmp("r-subject"), OFFSET).await.unwrap();
    let reader = Vault::open(tmp("r-reader"), OFFSET).await.unwrap();
    vault.register(&reader).await.unwrap();
    reader.register(&vault).await.unwrap();

    let subject = vault.subject().to_hex();
    let author = vault.subject();
    let mut published = vault.seal(&[]).await.unwrap();
    published.extend(vault.grant(reader.subject(), Reach::Everything).await.unwrap());
    for e in 0..3i64 {
        published.extend(vault.seal(&day(22_400 + e, 110.0 + e as f64)).await.unwrap());
    }

    let publisher = Swarm::join_network(tmp("r-pub"), SigningKey::generate(), net).await.unwrap();
    let (e, g) = publisher.parts();
    let publishing = Replicator::start(vault.store(), e, g).await.unwrap();

    // --- a stranger carries it. It is granted nothing and told nothing. ----
    let carrier_vault = Vault::open(tmp("r-carrier"), OFFSET).await.unwrap();
    let carrier = Swarm::join_network(tmp("r-car"), SigningKey::generate(), net).await.unwrap();
    let (e, g) = carrier.parts();
    let carrying = Replicator::start(carrier_vault.store(), e, g).await.unwrap();

    let depth = pool::depth_for(2);
    let topic = pool::bucket_topic(depth, pool::bucket_of(&subject, depth));
    publishing.carry(topic, &subject).await.unwrap();
    carrying.carry(topic, &subject).await.unwrap();

    for _ in 0..60 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if held(&carrier_vault.store(), &author).await as usize >= published.len() {
            break;
        }
    }
    let carried = held(&carrier_vault.store(), &author).await as usize;
    assert!(
        carried >= published.len(),
        "the carrier holds {carried} of {} operations, so there is nothing to relay",
        published.len()
    );

    // --- and the reader gets it FROM THE CARRIER ---------------------------
    //
    // Read out of the carrier's own store, not handed the subject's
    // operations: if the reader were given `published` directly this would
    // prove only what `tests/vault.rs` already proves.
    let relayed = operations_of(&carrier_vault.store(), &author).await;
    assert_eq!(
        relayed.len(),
        published.len(),
        "the carrier's copy is not the whole subject"
    );

    let got = reader.ingest(&relayed).await.unwrap();
    assert_eq!(got.panicked, 0, "an operation panicked while reading from the carrier");

    let want: Vec<String> = (0..3i64)
        .map(|e| day(22_400 + e, 110.0 + e as f64)[0].to_canonical_json())
        .collect();
    let read: Vec<String> = got.records.iter().map(|r| r.to_canonical_json()).collect();
    for line in &want {
        assert!(read.contains(line), "the reader did not get {line} from the carrier");
    }

    // And the carrier still cannot read a byte of what it just handed over.
    assert!(
        carrier_vault.ingest(&relayed).await.unwrap().records.is_empty(),
        "the carrier opened the data it was relaying"
    );
}
