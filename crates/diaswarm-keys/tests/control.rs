//! Can a grant travel, and can anything else pretend to be one?
//!
//! **THE DEFECT THIS CLOSES.** `diaswarm-keys` shipped with a module note in red:
//! `group::Message` is a plain struct with a settable `sender`, `Vault::receive`
//! processed one without checking, and `wire` carried segments but not control
//! messages — so there was no transport for a grant at all, and the thing that
//! would have delivered a forged one was the same thing that did not exist.
//!
//! Fixing the transport is what makes the authentication necessary rather than
//! theoretical, so both are asked here, in that order: first that a real grant
//! gets from a subject to a reader through a log and nothing else, then that
//! each of the three ways of faking one is refused.

use diaswarm_core::{EPOCH_MS, Record};
use diaswarm_keys::group::{GrantTag, Message};
use diaswarm_keys::wire::{self, ControlArgs, ControlOperation, CONTROL_V1};
use diaswarm_keys::{Error, Vault};
use p2panda_core::{Body, Header, SigningKey};
use p2panda_encryption::Rng;
use p2panda_store::SqliteStoreBuilder;

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
        "diaswarm-control-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn day(epoch: i64) -> Vec<Record> {
    (0..12i64)
        .map(|i| {
            Record::new(epoch * EPOCH_MS + i * 300_000, "cgm")
                .set("mgdl", Some((100.0 + i as f64).into()))
        })
        .collect()
}

/// A GRANT GETS THERE, CARRYING NOTHING BUT ITSELF.
///
/// The subject never hands the reader a `Message`. It publishes one to its
/// control log; the reader fetches that log and is given an `Authentic` only
/// because the signature held. Everything after — the welcome, the secret
/// bundle, the segments — follows from that one operation.
#[tokio::test(flavor = "multi_thread")]
async fn a_grant_travels_as_a_signed_operation_and_nothing_else() {
    let rng = Rng::default();
    let subject_key = SigningKey::from_bytes(&rand32());
    let reader_key = SigningKey::from_bytes(&rand32());
    let root = tmp("travel");
    let store = SqliteStoreBuilder::memory().build().await.expect("store");

    let mut subject = Vault::open(&root, OFFSET, &subject_key).expect("subject");
    let (subject_mgr, subject_bundle) = Vault::key_bundle(&rng).expect("bundle");
    let (reader_mgr, reader_bundle) = Vault::key_bundle(&rng).expect("bundle");
    subject.create(subject_mgr).expect("create");
    let (welcome, _tag) = subject.grant(reader_bundle, "follow").expect("grant");

    // The subject's whole side of the handover.
    wire::publish_control(&store, &subject_key, &welcome).await.expect("publish");

    // The reader's. It has the subject's key — from the invite — and the log.
    let mut arrived = wire::control_from(&store, &subject_key.verifying_key(), None)
        .await
        .expect("read control");
    assert_eq!(arrived.len(), 1, "expected exactly the welcome");
    let welcome = arrived.pop().expect("welcome");
    assert_eq!(welcome.author(), &subject_key.verifying_key());

    let mut reader = Vault::open(&root, OFFSET, &reader_key).expect("reader");
    let registry = Vault::registry(&[(subject.subject(), subject_bundle.clone())]).expect("registry");
    reader.join(reader_mgr, registry, &subject_bundle, "follow", &welcome).expect("join");

    subject.seal(20_000, &day(20_000)).expect("seal");
    let got = reader.read_from(i64::MIN).expect("read");
    assert_eq!(got.len(), 1, "the reader that joined from the wire read nothing");
}

/// THE THREE WAYS OF FAKING A GRANT, AND THREE REFUSALS.
#[tokio::test(flavor = "multi_thread")]
async fn a_control_message_must_prove_who_sent_it() {
    let rng = Rng::default();
    let subject_key = SigningKey::from_bytes(&rand32());
    let impostor_key = SigningKey::from_bytes(&rand32());
    let root = tmp("forge");
    let store = SqliteStoreBuilder::memory().build().await.expect("store");

    let mut subject = Vault::open(&root, OFFSET, &subject_key).expect("subject");
    let (subject_mgr, _bundle) = Vault::key_bundle(&rng).expect("bundle");
    let (_reader_mgr, reader_bundle) = Vault::key_bundle(&rng).expect("bundle");
    subject.create(subject_mgr).expect("create");
    let (welcome, _tag) = subject.grant(reader_bundle, "follow").expect("grant");

    // ---- 1. somebody else's signature over a real message ----
    //
    // The message is genuine — the subject built it. What is wrong is who
    // signed the operation carrying it, which is the case a network attacker
    // who has seen a grant go past is actually in.
    let payload = p2panda_core::cbor::encode_cbor(&welcome).expect("encode");
    let header = Header::builder()
        .seq_num(0)
        .body(&payload)
        .build(&impostor_key, ControlArgs { v: CONTROL_V1 });
    let forged = ControlOperation::from_parts(header, Some(Body::from_bytes(payload)));
    let err = wire::open_control(forged, &subject_key.verifying_key()).unwrap_err();
    assert!(matches!(err, Error::Forged(_)), "an impostor's signature was accepted: {err}");

    // ---- 2. the subject's own signature over a swapped body ----
    //
    // The header is genuinely the subject's and genuinely signed. Only the
    // payload has been replaced, which is what `validate_operation` exists to
    // notice — and what nothing in this crate was doing before.
    let real = wire::publish_control(&store, &subject_key, &welcome).await.expect("publish");
    let tampered = ControlOperation::from_parts(
        real.header.clone(),
        Some(Body::from_bytes(b"a different message entirely".to_vec())),
    );
    let err = wire::open_control(tampered, &subject_key.verifying_key()).unwrap_err();
    assert!(matches!(err, Error::Forged(_)), "a swapped body was accepted: {err}");

    // ---- 3. a message that lies about its sender ----
    //
    // `sender` is a field in a struct, so anyone holding a message can change
    // it. Signing it does not make it true, and the check is that the sender
    // equals `GrantTag::own` of whoever signed.
    let mut lying = welcome.clone();
    if let Message::Control { sender, .. } = &mut lying {
        *sender = GrantTag::own(&impostor_key.verifying_key());
    }
    let err = wire::publish_control(&store, &subject_key, &lying).await.unwrap_err();
    assert!(matches!(err, Error::Forged(_)), "a message lying about its sender was published: {err}");
}

/// A VALID GRANT FROM THE WRONG PERSON IS STILL NOT A GRANT.
///
/// A follower of two children holds two vaults. Both subjects sign real
/// control messages; neither one's grants belong in the other's vault. This is
/// the check `wire::open_control` cannot make on its own, because it is told
/// which key to expect and both keys are somebody's.
#[tokio::test(flavor = "multi_thread")]
async fn a_vault_refuses_a_genuine_message_from_a_different_subject() {
    let rng = Rng::default();
    let a_key = SigningKey::from_bytes(&rand32());
    let b_key = SigningKey::from_bytes(&rand32());
    let reader_key = SigningKey::from_bytes(&rand32());
    let store = SqliteStoreBuilder::memory().build().await.expect("store");

    // Subject A grants the reader, and the reader joins A's group.
    let mut a = Vault::open(tmp("subject-a"), OFFSET, &a_key).expect("a");
    let (a_mgr, a_bundle) = Vault::key_bundle(&rng).expect("bundle");
    let (reader_mgr, reader_bundle) = Vault::key_bundle(&rng).expect("bundle");
    a.create(a_mgr).expect("create");
    let (welcome, _tag) = a.grant(reader_bundle.clone(), "follow").expect("grant");
    wire::publish_control(&store, &a_key, &welcome).await.expect("publish a");
    let welcome = wire::control_from(&store, &a_key.verifying_key(), None)
        .await
        .expect("read a")
        .pop()
        .expect("welcome");

    let mut reader = Vault::open(tmp("reader"), OFFSET, &reader_key).expect("reader");
    let registry = Vault::registry(&[(a.subject(), a_bundle.clone())]).expect("registry");
    reader.join(reader_mgr, registry, &a_bundle, "follow", &welcome).expect("join");

    // Subject B signs a real control message of its own.
    let mut b = Vault::open(tmp("subject-b"), OFFSET, &b_key).expect("b");
    let (b_mgr, _b_bundle) = Vault::key_bundle(&rng).expect("bundle");
    b.create(b_mgr).expect("create");
    let (b_welcome, _tag) = b.grant(reader_bundle, "follow").expect("grant");
    wire::publish_control(&store, &b_key, &b_welcome).await.expect("publish b");
    let b_welcome = wire::control_from(&store, &b_key.verifying_key(), None)
        .await
        .expect("read b")
        .pop()
        .expect("welcome");

    // It authenticates — B really did sign it — and it is still refused.
    assert_eq!(b_welcome.author(), &b_key.verifying_key());
    let err = reader.receive(&b_welcome).unwrap_err();
    assert!(
        matches!(err, Error::Forged(_)),
        "a vault took a control message from a subject it does not follow: {err}"
    );
}

/// CONTROL MESSAGES MUST NOT LAND IN THE SEGMENT LOG.
///
/// `wire::segments_tail` asks for the last N entries and calls them the last N
/// days. If a grant were appended to the same log that stops being true, and
/// the symptom would be a follower quietly showing six days when it asked for
/// seven.
#[tokio::test(flavor = "multi_thread")]
async fn grants_and_segments_do_not_share_a_log() {
    let rng = Rng::default();
    let subject_key = SigningKey::from_bytes(&rand32());
    let root = tmp("logs");
    let store = SqliteStoreBuilder::memory().build().await.expect("store");

    let mut subject = Vault::open(&root, OFFSET, &subject_key).expect("subject");
    let (mgr, _bundle) = Vault::key_bundle(&rng).expect("bundle");
    let (_rm, reader_bundle) = Vault::key_bundle(&rng).expect("bundle");
    let create = subject.create(mgr).expect("create");
    wire::publish_control(&store, &subject_key, &create).await.expect("publish create");

    for e in 0..7i64 {
        let segment = subject.seal(20_000 + e, &day(20_000 + e)).expect("seal");
        wire::publish(&store, &subject_key, &segment).await.expect("publish segment");
        // A grant in the middle of the week, which is the interleaving that
        // would break the count if the two shared a log.
        if e == 3 {
            let (welcome, _tag) = subject.grant(reader_bundle.clone(), "follow").expect("grant");
            wire::publish_control(&store, &subject_key, &welcome).await.expect("publish grant");
        }
    }

    let author = subject_key.verifying_key();
    let week = wire::segments_tail(&store, &author, 7).await.expect("tail");
    assert_eq!(week.len(), 7, "seven entries of the segment log were not seven days");
    let epochs: Vec<i64> = week.iter().map(|s| s.epoch).collect();
    assert_eq!(epochs, (20_000..20_007).collect::<Vec<_>>(), "days came back wrong: {epochs:?}");

    let control = wire::control_from(&store, &author, None).await.expect("control");
    assert_eq!(control.len(), 2, "expected the create and the grant, got {}", control.len());
}
