//! Does p2panda-encryption's data mode give the property tools/seal.py demonstrates?
//!
//! tools/seal.py is the framework-neutral reference (decision D10): one content
//! key per UTC-day epoch, wrapped per grantee, granting starts the wrapping and
//! revoking stops it. This spike asks whether the library that will actually
//! ship gives the same guarantee, and at what granularity.
//!
//! THE QUESTIONS, from feasibility.md §10.2:
//!
//!   1. After removal, does the removed reader decrypt nothing new?
//!   2. Does everything they already held still open?
//!   3. Is revocation granularity the epoch, or something else?
//!   4. Can a joiner be given PART of the history — the claim §8.4 makes about
//!      withholding history from a researcher while granting it to a clinician?
//!
//! Run: cargo run --manifest-path spike/p2panda-seal/Cargo.toml

use p2panda_encryption::crypto::Rng;
use p2panda_encryption::crypto::x25519::SecretKey;
use p2panda_encryption::data_scheme::{
    EncryptionGroup, GroupOutput, GroupState, SecretBundle, decrypt_data,
};
use p2panda_encryption::key_bundle::Lifetime;
use p2panda_encryption::key_manager::KeyManager;
use p2panda_encryption::key_registry::KeyRegistry;
use p2panda_encryption::test_utils::data_scheme::dgm::TestDgm;
use p2panda_encryption::test_utils::data_scheme::ordering::{MessageOrderer, TestMessage};
use p2panda_encryption::test_utils::{MemberId, MessageId};
use p2panda_encryption::traits::{GroupMessage, GroupMessageContent, PreKeyManager};

type Dgm = TestDgm<MemberId, MessageId>;
type Ord_ = MessageOrderer<Dgm>;
type Group = EncryptionGroup<MemberId, MessageId, KeyRegistry<MemberId>, Dgm, KeyManager, Ord_>;
type State = GroupState<MemberId, MessageId, KeyRegistry<MemberId>, Dgm, KeyManager, Ord_>;

const SUBJECT: MemberId = 0;
const PARTNER: MemberId = 1;
const CLINIC: MemberId = 2;
const COHORT: MemberId = 3;
const WINDOWED: MemberId = 4;

struct Party {
    name: &'static str,
    id: MemberId,
    state: Option<State>,
    /// Every application message this party ever saw, kept so we can ask later
    /// whether what they already held still opens.
    seen: Vec<TestMessage<Dgm>>,
}

fn report(label: &str, pass: bool, detail: &str) {
    println!("    {:<38} {}   {}", label, if pass { "PASS" } else { "FAIL" }, detail);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rng = Rng::from_seed([7; 32]);

    // --- everyone publishes a long-term prekey bundle ----------------------
    let mut keys = Vec::new();
    let mut bundles = Vec::new();
    for _ in 0..5 {
        let secret = SecretKey::from_bytes(rng.random_array()?);
        let km = KeyManager::init_and_generate_prekey(&secret, Lifetime::default(), &rng)?;
        let bundle = KeyManager::prekey_bundle(&km)?;
        keys.push(km);
        bundles.push(bundle);
    }

    let registry = {
        let mut pki = KeyRegistry::init();
        for (id, bundle) in bundles.iter().enumerate() {
            pki = KeyRegistry::add_longterm_bundle(pki, id, bundle.clone())?;
        }
        pki
    };

    let mut parties: Vec<Party> = ["subject", "partner", "clinic", "cohort", "windowed"]
        .iter()
        .enumerate()
        .map(|(id, name)| Party {
            name,
            id,
            state: Some(Group::init(
                id,
                keys[id].clone(),
                registry.clone(),
                TestDgm::init(id),
                MessageOrderer::init(id),
            )),
            seen: Vec::new(),
        })
        .collect();

    println!("\n  p2panda-encryption 0.7.1, data_scheme\n");

    // --- the subject creates the group ------------------------------------
    let (state, welcome) = Group::create(
        parties[SUBJECT].state.take().unwrap(),
        vec![PARTNER, CLINIC],
        &rng,
    )?;
    parties[SUBJECT].state = Some(state);

    let mut broadcast: Vec<TestMessage<Dgm>> = vec![welcome];

    // Deliver everything pending to everyone but the sender.
    macro_rules! deliver {
        ($msgs:expr) => {{
            let mut pending: Vec<TestMessage<Dgm>> = $msgs;
            while let Some(msg) = pending.pop() {
                for p in parties.iter_mut() {
                    if msg.sender() == p.id {
                        continue;
                    }
                    // Clone rather than take: a removed member cannot process
                    // further control messages, and losing their state here
                    // would hide the very thing we are measuring.
                    let Some(state) = p.state.clone() else { continue };
                    match Group::receive(state, &msg) {
                        Ok((next, out)) => {
                            p.state = Some(next);
                            for o in out {
                                match o {
                                    GroupOutput::Control(m) => pending.push(m),
                                    GroupOutput::Removed => {
                                        println!("    {} was told it is removed", p.name)
                                    }
                                    GroupOutput::Application { .. } => {}
                                }
                            }
                        }
                        Err(_) => {}
                    }
                }
            }
        }};
    }
    deliver!(std::mem::take(&mut broadcast));

    // --- seal a day at a time, rotating the key at each epoch boundary ----
    // This is the epoch construction expressed in p2panda's terms: update()
    // rotates without a membership change, which is what a daily epoch needs.
    let mut sealed: Vec<(usize, TestMessage<Dgm>)> = Vec::new();
    let mut rotations = 0usize;

    for epoch in 0..10usize {
        if epoch > 0 {
            let (state, msg) = Group::update(parties[SUBJECT].state.take().unwrap(), &rng)?;
            parties[SUBJECT].state = Some(state);
            rotations += 1;
            deliver!(vec![msg]);
        }

        if epoch == 6 {
            // THE REVOCATION, landing with history still to come.
            let (state, msg) = Group::remove(parties[SUBJECT].state.take().unwrap(), PARTNER, &rng)?;
            parties[SUBJECT].state = Some(state);
            deliver!(vec![msg]);
        }

        let day = format!("epoch {epoch}: cgm 100.{epoch}");
        let (state, msg) = Group::send(parties[SUBJECT].state.take().unwrap(), day.as_bytes(), &rng)?;
        parties[SUBJECT].state = Some(state);
        sealed.push((epoch, msg.clone()));

        // Each reader tries to open it, and keeps it either way — a peer holds
        // the ciphertext whether or not it can read it.
        for p in parties.iter_mut().filter(|p| p.id != SUBJECT) {
            p.seen.push(msg.clone());
            if let Some(state) = p.state.clone() {
                if let Ok((next, _)) = Group::receive(state, &msg) {
                    p.state = Some(next);
                }
            }
        }
    }

    // --- what each reader can open NOW, from the keys it still holds -------
    //
    // Not by replaying through receive(): the orderer refuses a message it has
    // already seen, so a replay would report nothing for everyone and look like
    // a revocation that worked. The honest question is the keys a reader still
    // holds against the bytes it still holds, so ask that directly.
    let mut opened: Vec<(usize, Vec<usize>)> = Vec::new();
    for p in parties.iter().filter(|p| p.id != SUBJECT) {
        let mut can = Vec::new();
        if let Some(state) = &p.state {
            for (epoch, msg) in &sealed {
                let GroupMessageContent::Application {
                    ciphertext,
                    group_secret_id,
                    nonce,
                } = msg.content()
                else {
                    continue;
                };
                if let Some(secret) = state.secrets.get(&group_secret_id) {
                    if decrypt_data(&ciphertext, secret, nonce).is_ok() {
                        can.push(*epoch);
                    }
                }
            }
        }
        println!(
            "    {:<9} holds {} secrets, opens epochs {:?}",
            p.name,
            p.state.as_ref().map(|s| s.secrets.len()).unwrap_or(0),
            can
        );
        opened.push((p.id, can));
    }

    let partner_opens = opened[0].1.clone();
    let clinic_opens = opened[1].1.clone();
    let partner_opens = &partner_opens;
    let clinic_opens = &clinic_opens;

    println!("\n  the property\n");
    report(
        "nothing new after removal",
        partner_opens.iter().all(|e| *e < 6),
        &format!("partner opens {:?}", partner_opens),
    );
    report(
        "everything already held still opens",
        (0..6).all(|e| partner_opens.contains(&e)),
        &format!("{} of 6 pre-removal epochs", partner_opens.iter().filter(|e| **e < 6).count()),
    );
    report(
        "revocation is per-recipient",
        clinic_opens.len() == 10,
        &format!("clinic opens {} of 10", clinic_opens.len()),
    );
    println!("\n  key rotation: {rotations} manual update()s, one per epoch boundary");

    // --- Q4: can a late joiner be given only PART of the history? ----------
    //
    // feasibility.md §8.4 claims the design "can withhold history from a
    // researcher while granting it to a clinician", because history-on-join is
    // a choice per join. add() takes no subset argument, so ask what a joiner
    // actually receives.
    println!("\n  a cohort member joins at the end, having been in no epoch\n");

    let (state, msg) = Group::add(parties[SUBJECT].state.take().unwrap(), COHORT, &rng)?;
    parties[SUBJECT].state = Some(state);
    deliver!(vec![msg]);

    let cohort_secrets = parties[COHORT].state.as_ref().map(|s| s.secrets.len()).unwrap_or(0);
    let subject_secrets = parties[SUBJECT].state.as_ref().unwrap().secrets.len();

    let mut cohort_opens = Vec::new();
    if let Some(state) = &parties[COHORT].state {
        for (epoch, msg) in &sealed {
            let GroupMessageContent::Application { ciphertext, group_secret_id, nonce } =
                msg.content() else { continue };
            if let Some(secret) = state.secrets.get(&group_secret_id) {
                if decrypt_data(&ciphertext, secret, nonce).is_ok() {
                    cohort_opens.push(*epoch);
                }
            }
        }
    }

    println!("    cohort    holds {cohort_secrets} of the subject's {subject_secrets} secrets");
    println!("    cohort    opens epochs {:?}", cohort_opens);
    report(
        "history on join is a per-join CHOICE",
        cohort_opens.len() < sealed.len(),
        &format!("opened {} of {} epochs it was never present for", cohort_opens.len(), sealed.len()),
    );

    // --- and can the subject force a window by trimming its own bundle? ---
    //
    // update_secrets() lets an application drop secrets from its OWN bundle.
    // If add() welcomes the joiner with whatever the adder happens to hold,
    // then trimming before the add and restoring afterwards would scope the
    // grant. Worth knowing whether that works before designing around it.
    println!("\n  a second joiner, added while the subject holds a trimmed bundle\n");

    let full = parties[SUBJECT].state.as_ref().unwrap().secrets.clone();
    let newest: Vec<_> = {
        let mut all: Vec<_> = full.secrets().cloned().collect();
        all.sort_by_key(|s| s.timestamp());
        all.into_iter().rev().take(2).collect()
    };

    let trimmed = Group::update_secrets(parties[SUBJECT].state.take().unwrap(), |_| {
        SecretBundle::from_secrets(newest)
    });
    parties[SUBJECT].state = Some(trimmed);

    let (state, msg) = Group::add(parties[SUBJECT].state.take().unwrap(), WINDOWED, &rng)?;
    parties[SUBJECT].state = Some(state);
    deliver!(vec![msg]);

    // Give the subject its history back.
    let restored = Group::update_secrets(parties[SUBJECT].state.take().unwrap(), |_| full);
    parties[SUBJECT].state = Some(restored);

    let mut windowed_opens = Vec::new();
    if let Some(state) = &parties[WINDOWED].state {
        for (epoch, msg) in &sealed {
            let GroupMessageContent::Application { ciphertext, group_secret_id, nonce } =
                msg.content() else { continue };
            if let Some(secret) = state.secrets.get(&group_secret_id) {
                if decrypt_data(&ciphertext, secret, nonce).is_ok() {
                    windowed_opens.push(*epoch);
                }
            }
        }
    }
    println!("    windowed  holds {} secrets, opens epochs {:?}",
        parties[WINDOWED].state.as_ref().map(|s| s.secrets.len()).unwrap_or(0),
        windowed_opens);
    report(
        "trimming the bundle scopes the grant",
        !windowed_opens.is_empty() && windowed_opens.len() < sealed.len(),
        &format!("opened {} of {}", windowed_opens.len(), sealed.len()),
    );

    Ok(())
}
