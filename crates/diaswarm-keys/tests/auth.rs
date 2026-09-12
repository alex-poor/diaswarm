//! Is a subject's record of its own grants one it can quietly revise?
//!
//! **D13'S TAMPER-EVIDENCE, ASKED OF D26'S VAULT.** That decision wanted a
//! grant log that is signed per entry, hash-chained so an altered or removed
//! entry shows up from a single copy, and replicated so that truncating the
//! local copy becomes equivocation rather than deletion. A p2panda log is all
//! of that by construction — which is exactly why it was worth testing rather
//! than asserting, because "by construction" had meant "and therefore nobody
//! checks it": `validate_operation` verifies a signature and a payload hash and
//! says nothing about whether one entry follows another.
//!
//! The three questions here are the three D13 asks, in its order: does an
//! intact log verify, is a doctored one caught from one copy, and is a
//! *plausible* one caught by comparing two.

use diaswarm_core::{EPOCH_MS, Record};
use diaswarm_keys::auth::{self, Agreement};
use diaswarm_keys::wire::{self, ControlArgs, KeysArgs, KeysOperation, CONTROL_LOG_ID, CONTROL_V1};
use diaswarm_keys::Vault;
use p2panda_core::{Body, Header, SigningKey};
use p2panda_encryption::Rng;
use p2panda_store::operations::OperationStore;
use p2panda_store::{SqliteStore, SqliteStoreBuilder, tx};

const OFFSET: i64 = 12 * 3_600_000;

fn rand32() -> [u8; 32] {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(&n.to_le_bytes());
    out[16..].copy_from_slice(&(n ^ 0x9E37_79B9_7F4A_7C15).to_le_bytes());
    out
}

fn tmp(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-auth-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn day(epoch: i64) -> Vec<Record> {
    vec![Record::new(epoch * EPOCH_MS, "cgm").set("mgdl", Some(120.0.into()))]
}

/// A subject, its store, and a control log with `grants + 1` entries in it.
async fn subject_with_grants(
    tag: &str,
    grants: usize,
) -> (SigningKey, Vault, SqliteStore) {
    let rng = Rng::default();
    let key = SigningKey::from_bytes(&rand32());
    let mut vault = Vault::open(tmp(tag), OFFSET, &key).expect("vault");
    let (mgr, _bundle) = Vault::key_bundle(&rng).expect("bundle");
    let store = SqliteStoreBuilder::memory().build().await.expect("store");

    let create = vault.create(mgr).expect("create");
    wire::publish_control(&store, &key, &create).await.expect("publish create");
    for _ in 0..grants {
        let (_m, reader_bundle) = Vault::key_bundle(&rng).expect("bundle");
        let (welcome, _t) = vault.grant(reader_bundle, "follow").expect("grant");
        wire::publish_control(&store, &key, &welcome).await.expect("publish grant");
    }
    (key, vault, store)
}

/// AN HONEST LOG VERIFIES, AND THE CHAIN IS ACTUALLY A CHAIN.
#[tokio::test(flavor = "multi_thread")]
async fn a_subjects_own_grant_log_checks_out() {
    let (key, _vault, store) = subject_with_grants("intact", 3).await;
    let chain = auth::verify_control_chain(&store, &key.verifying_key()).await.expect("verify");

    assert!(chain.is_intact(), "an honest log reported a break at {:?}", chain.broken_at);
    assert_eq!(chain.len(), 4, "expected the create and three grants");
    assert!(chain.head().is_some());

    // Every entry is distinct, which is the thing `Message::stamp`'s empty-bytes
    // fallback used to destroy — one id for every message in the group.
    let mut seen = chain.entries.clone();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), chain.len(), "two entries in the log have the same hash");
}

/// AN ENTRY REMOVED FROM THE MIDDLE IS CAUGHT FROM ONE COPY.
///
/// **D13'S EXACT WORDING**: a per-entry signature cannot notice that an entry
/// is gone, because every entry that remains is still perfectly signed. The
/// chain notices, because the entry after the hole says which entry it follows
/// and that entry is not there.
///
/// Built by copying an honest log entry by entry and skipping one, which is
/// what a subject handing out an edited history would actually produce — not a
/// corrupted file, a shorter clean one.
#[tokio::test(flavor = "multi_thread")]
async fn an_entry_removed_from_the_middle_is_caught() {
    let (key, _vault, honest) = subject_with_grants("removed", 2).await;
    let subject = key.verifying_key();
    let full = auth::verify_control_chain(&honest, &subject).await.expect("verify");
    assert!(full.is_intact());
    assert_eq!(full.len(), 3);

    // The same log with entry 1 left out.
    let edited = SqliteStoreBuilder::memory().build().await.expect("store");
    for (i, operation) in control_log(&honest, &subject).await.into_iter().enumerate() {
        if i == 1 {
            continue;
        }
        put(&edited, &operation).await;
    }

    let chain = auth::verify_control_chain(&edited, &subject).await.expect("verify");
    assert_eq!(
        chain.broken_at,
        Some(1),
        "an entry taken out of the middle was not caught: {chain:?}"
    );

    // And a reader fetching that log is refused rather than handed it.
    let err = wire::control_from(&edited, &subject, None).await.unwrap_err();
    assert!(format!("{err}").contains("chain"), "control_from served a broken log: {err}");
}

/// AND A RE-SIGNED ENTRY AT A POSITION ALREADY TAKEN.
///
/// The other shape of the same lie: rather than remove an entry, sign a second
/// one claiming the same place in the log. It is genuinely the subject's
/// signature and genuinely sequence number 2 — there is simply already a
/// sequence number 2.
#[tokio::test(flavor = "multi_thread")]
async fn a_second_entry_claiming_a_taken_position_is_caught() {
    let (key, _vault, store) = subject_with_grants("duplicate", 2).await;
    let subject = key.verifying_key();
    assert!(auth::verify_control_chain(&store, &subject).await.unwrap().is_intact());

    let payload = b"a grant that was never made".to_vec();
    let header = Header::builder()
        .seq_num(2)
        .backlink(Some(p2panda_core::Hash::digest(b"whatever came before")))
        .body(&payload)
        .build(&key, KeysArgs::Control(ControlArgs { v: CONTROL_V1 }));
    let doctored = KeysOperation::from_parts(header, Some(Body::from_bytes(payload)));
    put(&store, &doctored).await;

    // The honest prefix still verifies, and the walk stops at the point where
    // the log stops being a line.
    let chain = auth::verify_control_chain(&store, &subject).await.expect("verify");
    assert_eq!(
        chain.broken_at,
        Some(3),
        "a second entry at a taken position was not caught: {chain:?}"
    );
    assert_eq!(chain.len(), 3, "the honest prefix should still verify");

    let err = wire::control_from(&store, &subject, None).await.unwrap_err();
    assert!(format!("{err}").contains("chain"), "control_from served a forked log: {err}");
}

/// TWO STORIES FROM ONE SUBJECT, AND NEITHER COPY CAN TELL ALONE.
///
/// **THE HALF OF D13 THAT NEEDS THE SWARM.** Both logs here are internally
/// perfect: signed throughout, sequence numbers in order, every backlink
/// correct. A peer holding either one verifies it and finds nothing wrong,
/// because there is nothing wrong with it — the subject simply signed two
/// different histories and showed one to each.
///
/// That is only visible by comparison, which is why `Chain` keeps every entry
/// hash rather than reducing to a head: a head alone cannot tell "this peer has
/// seen less" from "this peer was told something else".
#[tokio::test(flavor = "multi_thread")]
async fn a_subject_showing_two_histories_is_caught_by_comparing_peers() {
    let rng = Rng::default();
    let key = SigningKey::from_bytes(&rand32());
    let subject = key.verifying_key();
    let mut vault = Vault::open(tmp("fork"), OFFSET, &key).expect("vault");
    let (mgr, _bundle) = Vault::key_bundle(&rng).expect("bundle");

    // Two peers, each holding this subject's control log.
    let alice = SqliteStoreBuilder::memory().build().await.expect("store");
    let bob = SqliteStoreBuilder::memory().build().await.expect("store");

    // A shared prefix: the group's creation, published to both.
    let create = vault.create(mgr).expect("create");
    for store in [&alice, &bob] {
        wire::publish_control(store, &key, &create).await.expect("publish create");
    }

    // Then a grant told only to Alice, and a different one told only to Bob.
    let (_m, first) = Vault::key_bundle(&rng).expect("bundle");
    let (to_alice, _t) = vault.grant(first, "follow").expect("grant");
    wire::publish_control(&alice, &key, &to_alice).await.expect("publish to alice");

    let (_m, second) = Vault::key_bundle(&rng).expect("bundle");
    let (to_bob, _t) = vault.grant(second, "clinician").expect("grant");
    wire::publish_control(&bob, &key, &to_bob).await.expect("publish to bob");

    // Each copy is beyond reproach on its own.
    let a = auth::verify_control_chain(&alice, &subject).await.expect("verify alice");
    let b = auth::verify_control_chain(&bob, &subject).await.expect("verify bob");
    assert!(a.is_intact(), "alice's copy broke: {:?}", a.broken_at);
    assert!(b.is_intact(), "bob's copy broke: {:?}", b.broken_at);
    assert_eq!(a.len(), 2);
    assert_eq!(b.len(), 2);

    // Together they are not.
    assert_eq!(
        a.compare(&b),
        Agreement::Forked { at: 1 },
        "equivocation between two peers went unnoticed"
    );

    // And a peer that has simply seen less is NOT an accusation.
    let behind = auth::Chain { entries: a.entries[..1].to_vec(), broken_at: None };
    assert_eq!(
        a.compare(&behind),
        Agreement::Consistent { shared: 1 },
        "a peer that is merely behind was reported as equivocation"
    );
}

/// A SECOND KEY IS A SECOND SUBJECT, NOT A SECOND VOICE.
///
/// "Who may grant" has a one-line answer here: the author of the log. There is
/// no `p2panda-auth` underneath, so there is no way to express "this other key
/// may also grant on my behalf" — and that is a limit worth writing down rather
/// than discovering. A second device is a second subject.
#[tokio::test(flavor = "multi_thread")]
async fn nobody_but_the_subject_writes_the_subjects_log() {
    let (key, mut vault, store) = subject_with_grants("author", 1).await;
    let subject = key.verifying_key();
    let other = SigningKey::from_bytes(&rand32());

    // The subject's own second device, signing a real grant with its own key.
    let rng = Rng::default();
    let (_m, bundle) = Vault::key_bundle(&rng).expect("bundle");
    let (welcome, _t) = vault.grant(bundle, "follow").expect("grant");
    let err = wire::publish_control(&store, &other, &welcome).await.unwrap_err();
    assert!(
        format!("{err}").contains("from"),
        "another key published into the subject's group: {err}"
    );

    // The subject's log is unchanged and still intact.
    let chain = auth::verify_control_chain(&store, &subject).await.expect("verify");
    assert!(chain.is_intact());
    assert_eq!(chain.len(), 2, "the log grew: {}", chain.len());

    // And the other key's own log is simply empty — a different subject, not a
    // co-author of this one.
    let theirs = auth::verify_control_chain(&store, &other.verifying_key())
        .await
        .expect("verify");
    assert!(theirs.is_empty(), "the other key has a log of its own in this group");

    // Sealing still works: none of this disturbed the vault.
    vault.seal(24_000, &day(24_000)).expect("seal");
}

/// Every operation in a subject's control log, in sequence order.
async fn control_log(store: &SqliteStore, subject: &p2panda_core::VerifyingKey) -> Vec<KeysOperation> {
    use p2panda_store::logs::LogStore;
    <SqliteStore as LogStore<KeysOperation, p2panda_core::VerifyingKey, u32, u32, p2panda_core::Hash>>::get_log_entries(
        store, subject, &CONTROL_LOG_ID, None, None,
    )
    .await
    .expect("read the control log")
    .into_iter()
    .flatten()
    .map(|(op, _encoded_header)| op)
    .collect()
}

/// Put an operation into a store's control log.
///
/// EVERY WRITE NEEDS A PERMIT, including one that looks self-contained: without
/// `tx!` this fails with "tried to interact with inexistant transaction", which
/// reads like a corrupt store rather than a macro left off.
async fn put(store: &SqliteStore, operation: &KeysOperation) {
    let out: Result<(), p2panda_store::SqliteError> = async {
        tx!(store, { store.insert_operation(&operation.hash, operation, &CONTROL_LOG_ID).await? });
        Ok(())
    }
    .await;
    out.expect("insert an operation");
}
