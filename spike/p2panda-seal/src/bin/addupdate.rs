//! Does a member added by `add()` keep receiving keys from later `update()`s?
//!
//! The smallest case that can answer it, because `windows.rs` measured a reader
//! that got the history bundle on being added and then never advanced again —
//! and that could equally be the library, the test DGM, or the way that spike
//! delivers messages. Attributing a finding to the wrong one of those is worse
//! than not having it.
//!
//!   create(subject, [bob]) → seal 0 → add(alice) → update → seal 1 → update → seal 2
//!
//! Bob is an initial member, alice an added one, and the only difference between
//! them is how they got in. If alice opens 1 and 2, the library is fine and
//! `windows.rs` has a bug. If she does not, an added member is a second-class
//! member and the whole "grant with history" design has to account for it.
//!
//! Run: cargo run --manifest-path spike/p2panda-seal/Cargo.toml --bin addupdate

use p2panda_encryption::crypto::Rng;
use p2panda_encryption::crypto::x25519::SecretKey;
use p2panda_encryption::data_scheme::{
    EncryptionGroup, GroupOutput, GroupState, decrypt_data,
};
use p2panda_encryption::key_bundle::Lifetime;
use p2panda_encryption::key_manager::{KeyManager, KeyManagerState};
use p2panda_encryption::key_registry::{KeyRegistry, KeyRegistryState};
use p2panda_encryption::test_utils::data_scheme::dgm::TestDgm;
use p2panda_encryption::test_utils::data_scheme::ordering::{MessageOrderer, TestMessage};
use p2panda_encryption::test_utils::{MemberId, MessageId};
use p2panda_encryption::traits::{GroupMessage, GroupMessageContent, PreKeyManager};

type Dgm = TestDgm<MemberId, MessageId>;
type Ord_ = MessageOrderer<Dgm>;
type Group = EncryptionGroup<MemberId, MessageId, KeyRegistry<MemberId>, Dgm, KeyManager, Ord_>;
type State = GroupState<MemberId, MessageId, KeyRegistry<MemberId>, Dgm, KeyManager, Ord_>;

const SUBJECT: MemberId = 0;
const BOB: MemberId = 1;
const ALICE: MemberId = 2;
const NAMES: [&str; 3] = ["subject", "bob", "alice"];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rng = Rng::from_seed([23; 32]);

    let mut kms: Vec<KeyManagerState> = Vec::new();
    let mut bundles = Vec::new();
    for _ in 0..NAMES.len() {
        let secret = SecretKey::from_bytes(rng.random_array()?);
        let km = KeyManager::init_and_generate_prekey(&secret, Lifetime::default(), &rng)?;
        bundles.push(KeyManager::prekey_bundle(&km)?);
        kms.push(km);
    }
    let registry: KeyRegistryState<MemberId> = {
        let mut pki = KeyRegistry::init();
        for (id, b) in bundles.iter().enumerate() {
            pki = KeyRegistry::add_longterm_bundle(pki, id, b.clone())?;
        }
        pki
    };

    let fresh = |id: MemberId| -> State {
        Group::init(
            id,
            kms[id].clone(),
            registry.clone(),
            TestDgm::init(id),
            MessageOrderer::init(id),
        )
    };

    // Everyone exists from the start and processes everything, so nothing here
    // depends on catch-up, replay or delivery order.
    let mut states: Vec<Option<State>> = (0..NAMES.len()).map(|id| Some(fresh(id))).collect();
    let mut log: Vec<TestMessage<Dgm>> = Vec::new();
    let mut refused = 0usize;

    macro_rules! broadcast {
        ($msgs:expr) => {{
            let mut pending: Vec<TestMessage<Dgm>> = $msgs;
            while let Some(msg) = pending.pop() {
                log.push(msg.clone());
                for id in 0..NAMES.len() {
                    if msg.sender() == id {
                        continue;
                    }
                    let Some(state) = states[id].clone() else { continue };
                    match Group::receive(state, &msg) {
                        Ok((next, out)) => {
                            states[id] = Some(next);
                            for o in out {
                                if let GroupOutput::Control(m) = o {
                                    pending.push(m);
                                }
                            }
                        }
                        Err(e) => {
                            refused += 1;
                            eprintln!("      refused by {}: {e}", NAMES[id]);
                        }
                    }
                }
            }
        }};
    }

    let (state, welcome) = Group::create(states[SUBJECT].take().unwrap(), vec![BOB], &rng)?;
    states[SUBJECT] = Some(state);
    broadcast!(vec![welcome]);
    println!("\n  create(subject, [bob])");

    let mut sealed: Vec<(usize, TestMessage<Dgm>)> = Vec::new();
    let seal = |n: usize,
                    states: &mut Vec<Option<State>>,
                    sealed: &mut Vec<(usize, TestMessage<Dgm>)>|
     -> Result<TestMessage<Dgm>, Box<dyn std::error::Error>> {
        let (state, msg) = Group::send(
            states[SUBJECT].take().unwrap(),
            format!("day {n}").as_bytes(),
            &rng,
        )?;
        states[SUBJECT] = Some(state);
        sealed.push((n, msg.clone()));
        Ok(msg)
    };

    let m = seal(0, &mut states, &mut sealed)?;
    broadcast!(vec![m]);

    let (state, msg) = Group::add(states[SUBJECT].take().unwrap(), ALICE, &rng)?;
    states[SUBJECT] = Some(state);
    broadcast!(vec![msg]);
    println!("  add(alice)");

    for n in 1..3usize {
        let (state, msg) = Group::update(states[SUBJECT].take().unwrap(), &rng)?;
        states[SUBJECT] = Some(state);
        broadcast!(vec![msg]);
        let m = seal(n, &mut states, &mut sealed)?;
        broadcast!(vec![m]);
    }
    println!("  update + seal, twice\n");

    for id in [BOB, ALICE] {
        let mut can = Vec::new();
        let state = states[id].as_ref().unwrap();
        for (n, msg) in &sealed {
            let GroupMessageContent::Application { ciphertext, group_secret_id, nonce } =
                msg.content()
            else {
                continue;
            };
            if let Some(secret) = state.secrets.get(&group_secret_id) {
                if decrypt_data(&ciphertext, secret, nonce).is_ok() {
                    can.push(*n);
                }
            }
        }
        println!(
            "    {:<7} {} secrets, opens days {:?}",
            NAMES[id],
            state.secrets.len(),
            can
        );
    }
    println!("    subject {} secrets", states[SUBJECT].as_ref().unwrap().secrets.len());
    println!("    control messages refused: {refused}");

    let alice = states[ALICE].as_ref().unwrap().secrets.len();
    let subject = states[SUBJECT].as_ref().unwrap().secrets.len();
    println!(
        "\n  {}\n",
        if alice == subject {
            "an added member tracks later updates — windows.rs has the bug"
        } else {
            "AN ADDED MEMBER DOES NOT TRACK LATER UPDATES.\n  \
             Measured with the crate's test_utils DGM and orderer, which a shipping\n  \
             integration replaces — re-measure against p2panda-auth + p2panda-store\n  \
             before treating this as a limit rather than an artefact."
        }
    );

    Ok(())
}
