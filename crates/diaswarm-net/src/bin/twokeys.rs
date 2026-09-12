//! A grant crossing between two real phones, on the D26 vault.
//!
//! **WHAT `twophone` IS FOR THE SPACES VAULT, THIS IS FOR THE KEYS ONE — AND IT
//! ASKS A HARDER QUESTION.** That binary proved operations replicate: a subject
//! seals, a stranger carries, and the stranger cannot read. This one has to get
//! a *reader* to the point of reading, which needs the thing the invite does not
//! carry.
//!
//! `p2panda-encryption` agrees keys from a `LongTermKeyBundle`, and **both
//! sides need the other's before a grant can exist**. The subject grants
//! against the reader's bundle; the reader needs the subject's to derive the
//! same `GrantTag` and open its welcome. `diaswarm-core`'s invite carries an
//! X25519 key that serves the same purpose for the old vault and is useless
//! here. Until that is fixed there is no way to grant anybody on this vault,
//! which is why it blocks the whole cutover.
//!
//! **SO THE BUNDLES TRAVEL ON THE COMMAND LINE HERE, AND THAT IS THE POINT.**
//! Nothing is being smuggled: it is exactly the two hops the real thing will
//! use — the subject's bundle in the invite, the reader's in `Request::Offer`
//! (D25 added that request so one scan finishes the exchange both ways). Moving
//! them into those two fields is app plumbing. Whether the protocol works at
//! all is this.
//!
//! ```sh
//! # 1. on the follower: print a bundle, then stop
//! twokeys bundle /data/local/tmp/tk
//! # 2. on the publisher: create, grant that bundle, seal, serve
//! twokeys publish /data/local/tmp/tk <reader-bundle>
//! # 3. on the follower: replicate, join from the control log, read
//! twokeys follow /data/local/tmp/tk <subject-hex> <subject-bundle>
//! ```
//!
//! A binary rather than the plugin, deliberately, for the reason `twophone`
//! gives: it runs from `/data/local/tmp`, touches no app, and goes nowhere near
//! a pump.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use diaswarm_core::{EPOCH_MS, Record};
use diaswarm_keys::{Vault, decode_bundle, encode_bundle, wire};
use diaswarm_net::pool;
use diaswarm_net::replicate::KeysReplicator;
use diaswarm_net::swarm::{Swarm, network_id};
use p2panda_core::SigningKey;
use p2panda_store::{SqliteStore, SqliteStoreBuilder};

const OFFSET: i64 = 12 * 3_600_000;

/// Both phones must agree on this, and on nothing else.
const POOL: &str = "twokeys-check";

fn day(epoch: i64, mgdl: f64) -> Vec<Record> {
    (0..12i64)
        .map(|i| {
            Record::new(epoch * EPOCH_MS + i * 300_000, "cgm")
                .set("mgdl", Some((mgdl + i as f64).into()))
        })
        .collect()
}

/// The subject identity, kept on disk so a restart is the same person.
///
/// `twophone` generates one per run because it only ever needs a stranger's
/// operations to arrive. A grant is made *to* an identity, so this one has to
/// survive: a publisher that came back as somebody else would have granted a
/// reader that no longer has a subject.
fn signing_key(root: &std::path::Path) -> Result<SigningKey> {
    let path = root.join("subject.key");
    if let Ok(bytes) = std::fs::read(&path) {
        let bytes: [u8; 32] = bytes.try_into().map_err(|_| anyhow::anyhow!("32 bytes"))?;
        return Ok(SigningKey::from_bytes(&bytes));
    }
    let key = SigningKey::generate();
    std::fs::write(&path, key.as_bytes())?;
    Ok(key)
}

async fn open_store(root: &std::path::Path) -> Result<SqliteStore> {
    let url = format!("sqlite://{}", root.join("keys.sqlite").display());
    SqliteStoreBuilder::new()
        .database_url(&url)
        .create_database(true)
        .build()
        .await
        .map_err(|e| anyhow::anyhow!("opening the store: {e}"))
}

/// Open the vault, creating the group and publishing its `Create` if new.
async fn open_vault(
    root: &std::path::Path,
    signing: &SigningKey,
    store: &SqliteStore,
) -> Result<Vault> {
    let mut vault = Vault::open(root, OFFSET, signing).context("opening the vault")?;
    if !vault.is_welcomed() {
        let rng = diaswarm_keys::Rng::default();
        let (manager, _bundle) = Vault::key_bundle(&rng).context("generating an identity")?;
        let create = vault.create(manager).context("creating the group")?;
        // PUBLISHED, NOT DISCARDED. The control log is D13's grant log here, and
        // a log that starts at the first grant cannot show that nothing came
        // before it.
        wire::publish_control(store, signing, &create).await.context("publishing create")?;
    }
    Ok(vault)
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let role = args.next().unwrap_or_default();
    let root = args.next().unwrap_or_else(|| "/data/local/tmp/tk".to_string());
    let root = std::path::PathBuf::from(root);
    std::fs::create_dir_all(&root)?;

    match role.as_str() {
        "bundle" => bundle(&root).await,
        "publish" => publish(&root, args.next()).await,
        "follow" => follow(&root, args.next(), args.next()).await,
        _ => {
            eprintln!("usage: twokeys <bundle|publish|follow> [dir] [args…]");
            std::process::exit(2);
        }
    }
}

/// Print this device's key bundle and stop. The follower's half of the exchange.
async fn bundle(root: &std::path::Path) -> Result<()> {
    let signing = signing_key(root)?;
    let store = open_store(root).await?;
    let vault = open_vault(root, &signing, &store).await?;
    println!("SUBJECT={}", signing.verifying_key().to_hex());
    println!("BUNDLE={}", encode_bundle(&vault.my_bundle()?)?);
    Ok(())
}

/// Create, grant the reader whose bundle was passed in, seal, and serve.
async fn publish(root: &std::path::Path, reader_bundle: Option<String>) -> Result<()> {
    let Some(reader_bundle) = reader_bundle else {
        eprintln!("  the reader's bundle is needed: twokeys publish <dir> <reader-bundle>");
        std::process::exit(2);
    };
    let signing = signing_key(root)?;
    let store = open_store(root).await?;
    let mut vault = open_vault(root, &signing, &store).await?;

    println!("  subject {}", &signing.verifying_key().to_hex()[..16]);
    println!("SUBJECT={}", signing.verifying_key().to_hex());
    println!("BUNDLE={}", encode_bundle(&vault.my_bundle()?)?);

    // ---- the grant ----
    let reader = decode_bundle(&reader_bundle).context("that is not a key bundle")?;
    let (welcome, tag) = vault.grant(reader, "follow").context("granting")?;
    wire::publish_control(&store, &signing, &welcome).await.context("publishing the grant")?;
    println!("  granted {tag}");

    // ---- some days ----
    let mut sealed = 0usize;
    for e in 0..3i64 {
        let segment = vault.seal(24_000 + e, &day(24_000 + e, 100.0 + e as f64))?;
        wire::publish(&store, &signing, &segment).await.context("publishing a segment")?;
        sealed += 1;
    }
    println!("  sealed  {sealed} days");

    // ---- the pool ----
    let net = network_id(POOL);
    let swarm = Swarm::join_via(
        root.join("pool"),
        SigningKey::generate(),
        net,
        diaswarm_net::swarm::DEFAULT_RELAY,
    )
    .await
    .context("joining the pool")?;
    let (endpoint, gossip) = swarm.parts();
    let replicator = KeysReplicator::keys(store.clone(), endpoint, gossip).await?;

    let subject = signing.verifying_key().to_hex();
    let members = swarm.pool_members().await?.len().max(2);
    let depth = pool::depth_for(members);
    let topic = pool::bucket_topic(depth, pool::bucket_of(&subject, depth));
    replicator.carry(topic, &subject).await?;

    println!("\n  RUN THIS ON THE OTHER PHONE:");
    println!("    twokeys follow /data/local/tmp/tk {subject} {}\n", encode_bundle(&vault.my_bundle()?)?);
    println!("  serving (ctrl-c to stop)…");
    loop {
        tokio::time::sleep(Duration::from_secs(10)).await;
        let n = swarm.pool_members().await.map(|m| m.len()).unwrap_or(0);
        println!("    pool {n} peers, {} operations sent", replicator.received());
    }
}

/// Replicate the subject, find our welcome in the control log, and read.
async fn follow(
    root: &std::path::Path,
    subject: Option<String>,
    subject_bundle: Option<String>,
) -> Result<()> {
    let (Some(subject), Some(subject_bundle)) = (subject, subject_bundle) else {
        eprintln!("  usage: twokeys follow <dir> <subject-hex> <subject-bundle>");
        std::process::exit(2);
    };
    let signing = signing_key(root)?;
    let store = open_store(root).await?;
    // Our own identity, made when `bundle` ran. The manager inside it is what
    // the subject granted against, so it has to be the same one.
    let mine = open_vault(root, &signing, &store).await?;
    let my_manager = mine.manager_state()?;
    drop(mine);

    let author = {
        let bytes: Vec<u8> = (0..subject.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&subject[i..i + 2], 16))
            .collect::<Result<Vec<u8>, _>>()?;
        p2panda_core::VerifyingKey::from_bytes(
            &bytes.try_into().map_err(|_| anyhow::anyhow!("32 bytes"))?,
        )?
    };
    let subject_bundle = decode_bundle(&subject_bundle).context("that is not a key bundle")?;

    // ---- the pool ----
    let net = network_id(POOL);
    let swarm = Swarm::join_via(
        root.join("pool"),
        SigningKey::generate(),
        net,
        diaswarm_net::swarm::DEFAULT_RELAY,
    )
    .await
    .context("joining the pool")?;
    let (endpoint, gossip) = swarm.parts();
    let replicator = KeysReplicator::keys(store.clone(), endpoint, gossip).await?;
    let members = swarm.pool_members().await?.len().max(2);
    let depth = pool::depth_for(members);
    let topic = pool::bucket_topic(depth, pool::bucket_of(&subject, depth));
    replicator.carry(topic, &subject).await?;
    println!("  following {}…", &subject[..16]);

    // ---- wait for the control log, then join from it ----
    let started = Instant::now();
    let mut joined: Option<Vault> = None;
    for _ in 0..90 {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let control = match wire::control_from(&store, &author, None).await {
            Ok(c) => c,
            Err(e) => {
                println!("    control log not usable yet: {e}");
                continue;
            }
        };
        if control.is_empty() {
            continue;
        }
        // A READER FINDS ITS OWN WELCOME BY TRYING THEM. Nothing in a control
        // message says in clear who it is for — a grant that announced its
        // recipient would undo D13 — so the one that opens is ours.
        let registry = Vault::registry(&[(
            diaswarm_keys::group::GrantTag::own(&author),
            subject_bundle.clone(),
        )])?;
        for message in &control {
            let mut candidate = Vault::open(root.join("joined"), OFFSET, &signing)?;
            if candidate
                .join(my_manager.clone(), registry.clone(), &subject_bundle, "follow", message)
                .is_ok()
            {
                println!("    joined after {:.0}s, as {}", started.elapsed().as_secs_f64(), candidate.subject());
                joined = Some(candidate);
                break;
            }
        }
        if joined.is_some() {
            break;
        }
        println!("    {} control message(s), none of them ours yet", control.len());
    }

    let Some(reader) = joined else {
        println!("\n  NEVER GRANTED. events: {:?}", replicator.events());
        std::process::exit(1);
    };

    // ---- and the days ----
    let mut opened = 0usize;
    let mut records = 0usize;
    for _ in 0..30 {
        let segments = wire::segments_tail(&store, &author, 10).await.unwrap_or_default();
        opened = 0;
        records = 0;
        for segment in &segments {
            if let Ok((rs, _bad)) = reader.open_segment(segment) {
                opened += 1;
                records += rs.len();
            }
        }
        if opened > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    println!();
    if opened == 0 {
        println!("  GRANTED BUT READ NOTHING. events: {:?}", replicator.events());
        std::process::exit(1);
    }
    println!("  READ {records} records from {opened} segments, off a phone it was never introduced to");
    Ok(())
}
