//! Keys from p2panda, data in our own segments — does it hold?
//!
//! Three questions, in the order that matters:
//!
//!   1. Can a member seal an arbitrarily large payload under the group secret,
//!      outside any p2panda message?
//!   2. Does reading the RECENT end stay flat as history grows — the property
//!      `diaswarm-core` has, `p2panda-spaces` loses, and the flagship needs?
//!   3. Does removal still cut a reader off from what comes next?

use std::time::Instant;

use p2panda_encryption::Rng;
use p2panda_encryption::crypto::xchacha20::XAeadNonce;
use p2panda_encryption::data_scheme::{decrypt_data, encrypt_data};
use p2panda_encryption::data_scheme::test_utils::network::Network;
use p2panda_encryption::test_utils::MemberId;

/// A sealed day, exactly as it would sit on disk: ciphertext plus the two
/// things needed to open it, and nothing that refers to any other segment.
struct Segment {
    epoch: i64,
    nonce: XAeadNonce,
    ciphertext: Vec<u8>,
    secret_id: Vec<u8>,
}

/// A day of a five-minute sensor, canonical-ish, so the sizes are honest.
fn day(epoch: i64) -> Vec<u8> {
    let mut out = String::new();
    for i in 0..288 {
        out.push_str(&format!(
            "{{\"k\":\"cgm\",\"mgdl\":{},\"t\":{}}}\n",
            100 + (i % 80),
            epoch * 86_400_000 + i * 300_000
        ));
    }
    out.into_bytes()
}

const SUBJECT: MemberId = 0;
const READER: MemberId = 1;

fn main() {
    let rng = Rng::from_seed([7; 32]);
    let mut net = Network::new([SUBJECT, READER], Rng::from_seed([7; 32]));
    net.create(SUBJECT, vec![SUBJECT, READER]);
    net.process();

    // ---- 1. seal days as independent segments ----
    let mut segments: Vec<Segment> = Vec::new();
    let seal = |net: &Network, epoch: i64, rng: &Rng| -> Segment {
        let y = net.members.get(&SUBJECT).expect("subject");
        let secret = y.secrets.latest().expect("a group secret exists");
        let nonce: XAeadNonce = rng.random_array().expect("nonce");
        let plaintext = day(epoch);
        Segment {
            epoch,
            nonce,
            ciphertext: encrypt_data(&plaintext, secret, nonce).expect("encrypt"),
            secret_id: secret.id().to_vec(),
        }
    };

    for epoch in 0..180i64 {
        segments.push(seal(&net, 20_000 + epoch, &rng));
    }
    println!(
        "  sealed {} days, {} KB of ciphertext, largest segment {} bytes",
        segments.len(),
        segments.iter().map(|s| s.ciphertext.len()).sum::<usize>() / 1024,
        segments.iter().map(|s| s.ciphertext.len()).max().unwrap_or(0)
    );
    println!("  (a spaces Application message caps at 64 KB and chains; these do neither)");

    // ---- 2. does reading the recent end stay flat? ----
    println!();
    println!("  {:>10}  {:>16}  {:>16}", "days held", "newest day", "whole history");
    for held in [7usize, 30, 90, 180] {
        let view = &segments[..held];
        let y = net.members.get(&READER).expect("reader");

        let newest = view.last().expect("a segment");
        let t0 = Instant::now();
        let secret = y.secrets.get(&id_of(&newest.secret_id)).expect("secret by id");
        let out = decrypt_data(&newest.ciphertext, secret, newest.nonce).expect("decrypt");
        let recent = t0.elapsed();
        assert!(!out.is_empty());

        let t1 = Instant::now();
        let mut total = 0usize;
        for s in view {
            let secret = y.secrets.get(&id_of(&s.secret_id)).expect("secret by id");
            total += decrypt_data(&s.ciphertext, secret, s.nonce).expect("decrypt").len();
        }
        let whole = t1.elapsed();
        assert!(total > 0);

        println!(
            "  {:>10}  {:>13.4}ms  {:>13.4}ms",
            held,
            recent.as_secs_f64() * 1000.0,
            whole.as_secs_f64() * 1000.0
        );
    }

    // ---- 3. does removal still cut them off? ----
    println!();
    net.remove(SUBJECT, READER);
    net.process();
    let after = seal(&net, 21_000, &rng);
    let y = net.members.get(&READER).expect("reader");
    match y.secrets.get(&id_of(&after.secret_id)) {
        None => println!("  after removal: the reader does not hold the new secret — cut off"),
        Some(secret) => match decrypt_data(&after.ciphertext, secret, after.nonce) {
            Ok(_) => println!("  after removal: THE READER STILL OPENED IT — revocation is broken"),
            Err(_) => println!("  after removal: the reader holds a secret that does not open it"),
        },
    }
    let old = segments.last().expect("a segment");
    let kept = y
        .secrets
        .get(&id_of(&old.secret_id))
        .map(|s| decrypt_data(&old.ciphertext, s, old.nonce).is_ok())
        .unwrap_or(false);
    println!(
        "  and what they already had: {} — prospective revocation, as the design says",
        if kept { "still readable" } else { "unreadable" }
    );
}

/// The bytes a segment names its secret by, back into an id.
fn id_of(bytes: &[u8]) -> p2panda_encryption::data_scheme::GroupSecretId {
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes[..32]);
    out
}
