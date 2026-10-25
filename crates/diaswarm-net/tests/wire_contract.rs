//! THE WIRE CONTRACT — the bytes [`diaswarm_core::PROTOCOL_VERSION`] promises.
//!
//! Two peers meet only if they derive the same topics from a subject and frame
//! an invite the same way. Those are the bytes the three products — desktop
//! carrier, Ayni follower, AAPS plugin — must agree on, and they are built from
//! one core but shipped separately, so a change that moves them silently splits
//! the fleet (a half-upgraded swarm never errors; it just never meets).
//!
//! This pins those bytes to committed golden values. **If a change here fails:**
//! that is a wire-protocol break. Bump `PROTOCOL_VERSION`, update the goldens
//! below to the new output, and release all three products together. **If
//! nothing you touched should have moved these bytes, you have an accidental
//! wire break — fix the change, not the fixture.** The fixtures are arbitrary
//! fixed inputs; only their *stability* is meaningful.

use diaswarm_core::invite::Invite;
use diaswarm_core::PROTOCOL_VERSION;
use diaswarm_net::pool::{bucket_topic, control_topic, presence_topic, subject_topic};

// Fixed, meaningless-but-stable inputs. Never change these to make a test pass;
// changing them defeats the point (they would no longer pin anything).
const SUBJECT: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
const ENDPOINT: &str = "ffeeddccbbaa99887766554433221100ffeeddccbbaa99887766554433221100";
const PURPOSE: &str = "follow";
const RELAY: &str = "https://relay.test/path";
const KEYS: &str = "a1b2c3d4e5f6";
const HANDLE: &str = "Mum";

fn hex(b: [u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The protocol version is a deliberate gate, not a label: it only moves when
/// the golden bytes below move. Kept here so the number lives beside the bytes
/// it certifies, and so a reviewer of either sees the other.
#[test]
fn protocol_version_is_pinned() {
    assert_eq!(
        PROTOCOL_VERSION, 1,
        "PROTOCOL_VERSION moved — the golden wire fixtures in this file must move \
         with it (and all three products must ship together), or it must not move"
    );
}

/// Topic derivation (`pool.rs` domain separators + SHA-256). A change to any
/// separator string or the hashing re-routes every peer to a different topic.
#[test]
fn topic_derivation_is_pinned() {
    assert_eq!(
        hex(subject_topic(SUBJECT)),
        "9ccb4818bd97fd176dcd9310c282b0a652cb78e6789cbc378a12e622acfed613",
        "subject_topic derivation changed"
    );
    assert_eq!(
        hex(control_topic(SUBJECT)),
        "d904694919da63628b64b1e59934849a1fe8bff2be0e24d9fa272600baa82c29",
        "control_topic derivation changed"
    );
    assert_eq!(
        hex(bucket_topic(0, 0)),
        "717603959f97918d81cdbac07d0b194fde605a148be9909f330f4a22c78962a7",
        "bucket_topic derivation changed"
    );
    assert_eq!(
        hex(presence_topic()),
        "6601ce2f49eba6ca2ae69769f89f6db74a62224b5ef5d59d96d55c5f1b39fc99",
        "presence_topic derivation changed"
    );
}

/// Invite framing, every shape a subject can emit: v2 (no keys), v3 (keys, no
/// handle), v4 (keys + handle). The version digit and the trailing checksum are
/// both part of the contract — an old build keys off the digit and verifies the
/// checksum.
#[test]
fn invite_framing_is_pinned() {
    let base = Invite::new_via(SUBJECT, ENDPOINT, PURPOSE, RELAY).unwrap();

    // What `encode` emits today: v2 with no bundle, and the compact base64url
    // v5 as soon as there is one — handle or not.
    let v2 = base.clone().encode();
    let v5 = base.clone().with_keys(KEYS).unwrap().encode();
    let v5_named = base.clone().with_keys(KEYS).unwrap().with_handle(HANDLE).unwrap().encode();
    // The pre-v5 hex forms `encode_compat` still emits for an older follower.
    let v3 = base.clone().with_keys(KEYS).unwrap().encode_compat();
    let v4 = base.with_keys(KEYS).unwrap().with_handle(HANDLE).unwrap().encode_compat();

    assert_eq!(
        v2,
        "diaswarm:2:00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff:ffeeddccbbaa99887766554433221100ffeeddccbbaa99887766554433221100:follow:https%3a//relay.test/path:a42a641a",
        "v2 invite framing changed"
    );
    assert_eq!(
        v5,
        "diaswarm:5:ABEiM0RVZneImaq7zN3u_wARIjNEVWZ3iJmqu8zd7v8:_-7dzLuqmYh3ZlVEMyIRAP_u3cy7qpmId2ZVRDMiEQA:follow:https%3a//relay.test/path:obLD1OX2::9d15631e",
        "v5 invite framing changed"
    );
    assert_eq!(
        v5_named,
        "diaswarm:5:ABEiM0RVZneImaq7zN3u_wARIjNEVWZ3iJmqu8zd7v8:_-7dzLuqmYh3ZlVEMyIRAP_u3cy7qpmId2ZVRDMiEQA:follow:https%3a//relay.test/path:obLD1OX2:Mum:bd54771c",
        "v5 named invite framing changed"
    );
    assert_eq!(
        v3,
        "diaswarm:3:00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff:ffeeddccbbaa99887766554433221100ffeeddccbbaa99887766554433221100:follow:https%3a//relay.test/path:a1b2c3d4e5f6:e485add0",
        "v3 fallback framing changed"
    );
    assert_eq!(
        v4,
        "diaswarm:4:00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff:ffeeddccbbaa99887766554433221100ffeeddccbbaa99887766554433221100:follow:https%3a//relay.test/path:a1b2c3d4e5f6:Mum:c2c6c934",
        "v4 fallback framing changed"
    );

    // A peer reads back what a peer emits — the other half of the contract,
    // for both the modern invite and the hex fallback.
    for text in [&v5_named, &v4] {
        let round = Invite::parse(text).unwrap();
        assert_eq!(round.subject, SUBJECT);
        assert_eq!(round.keys, KEYS);
        assert_eq!(round.handle, HANDLE);
    }
}
