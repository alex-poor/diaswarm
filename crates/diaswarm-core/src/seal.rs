//! Epoch sealing: one content key per UTC day, wrapped to each live grantee.
//!
//! A port of `tools/seal.py`, and byte-compatible with it on purpose — the
//! interop tests seal in one language and open in the other, in both
//! directions. That is the only way to know two implementations of a
//! construction agree, and this project has already been bitten twice by
//! assuming they would.
//!
//! THE PROPERTY THIS EXISTS TO PROVIDE:
//!
//!   after revocation the reader decrypts NOTHING NEW, and everything they
//!   ALREADY HELD still opens
//!
//! Granting starts the wrapping and revoking stops it. There is no delete step
//! and no message to anyone: a revoked reader is simply one the loop no longer
//! wraps for, and what they already hold is untouched. That second half is not
//! a shortcoming to apologise for — claiming otherwise would be claiming a
//! recall this design cannot perform, which feasibility.md §11 forbids saying.
//!
//! NOT REVIEWED CRYPTOGRAPHY. X25519 + HKDF-SHA256 + ChaCha20-Poly1305 and
//! Ed25519 for grants, composed by hand. Stage 10.6 is an external review gate
//! and this module is exactly what it is for.
//!
//! NOT FORWARD-SECRET WITHIN AN EPOCH. A reader's compromised device exposes
//! every epoch they were ever wrapped, permanently — feasibility.md §7.4 names
//! that a permanent property of the architecture and "the sharpest thing to say
//! out loud".

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use rand_core::{OsRng, RngCore};
use sha2::Sha256;
use x25519_dalek::{EphemeralSecret, PublicKey, StaticSecret};

/// Bumping this changes every derived key, so it is a compatibility break in
/// the same way a record schema change is. It exists so a future scheme can be
/// told apart from this one rather than silently producing garbage.
pub const WRAP_INFO: &[u8] = b"diaswarm-wrap-v1";

pub const KEY_BYTES: usize = 32;
pub const NONCE_BYTES: usize = 12;

#[derive(Debug)]
pub enum SealError {
    /// Wrong key, wrong epoch, wrong subject, or tampered bytes. Deliberately
    /// one variant: distinguishing them for a caller would tell an attacker
    /// which guess was closer.
    Undecryptable,
    Malformed,
}

/// One epoch's content key. Independent of every other epoch's.
///
/// Independence is the whole construction. A chain, a ratchet or a KDF from a
/// master would mean that handing a reader epoch 40 tells them something about
/// epoch 41, and revocation would stop meaning what it says.
pub fn epoch_key() -> [u8; KEY_BYTES] {
    let mut k = [0u8; KEY_BYTES];
    OsRng.fill_bytes(&mut k);
    k
}

fn derive(shared: &[u8], info: &[u8], context: &[u8]) -> [u8; KEY_BYTES] {
    let mut full = Vec::with_capacity(info.len() + context.len());
    full.extend_from_slice(info);
    full.extend_from_slice(context);
    let hk = Hkdf::<Sha256>::new(None, shared);
    let mut out = [0u8; KEY_BYTES];
    hk.expand(&full, &mut out).expect("32 bytes is a valid HKDF length");
    out
}

/// The bytes authenticated but not encrypted alongside a sealed epoch.
///
/// A holder must be able to route and replicate a sealed epoch without reading
/// it, and must not be able to pass epoch 12 off as epoch 13 to a reader who
/// was only granted 13.
fn seal_aad(subject: &[u8; 32], epoch: i64) -> Vec<u8> {
    let mut aad = Vec::with_capacity(40);
    aad.extend_from_slice(subject);
    aad.extend_from_slice(&epoch.to_be_bytes());
    aad
}

/// Seal one epoch's already-encoded records.
pub fn seal_epoch(
    plaintext: &[u8],
    key: &[u8; KEY_BYTES],
    epoch: i64,
    subject: &[u8; 32],
) -> Vec<u8> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let mut nonce_bytes = [0u8; NONCE_BYTES];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let aad = seal_aad(subject, epoch);
    let ct = cipher
        .encrypt(nonce, Payload { msg: plaintext, aad: &aad })
        .expect("encryption does not fail for a valid key and nonce");
    let mut out = Vec::with_capacity(NONCE_BYTES + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    out
}

/// Open a sealed epoch. Fails if the key is wrong or the epoch was moved.
pub fn open_epoch(
    sealed: &[u8],
    key: &[u8; KEY_BYTES],
    epoch: i64,
    subject: &[u8; 32],
) -> Result<Vec<u8>, SealError> {
    if sealed.len() < NONCE_BYTES {
        return Err(SealError::Malformed);
    }
    let (nonce_bytes, ct) = sealed.split_at(NONCE_BYTES);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let aad = seal_aad(subject, epoch);
    cipher
        .decrypt(Nonce::from_slice(nonce_bytes), Payload { msg: ct, aad: &aad })
        .map_err(|_| SealError::Undecryptable)
}

/// Wrap an epoch key to one reader. 92 bytes.
///
/// Ephemeral-static X25519: a fresh ephemeral key per wrap, so the subject's
/// long-term key is never the only thing between a reader and every epoch.
pub fn wrap(key: &[u8; KEY_BYTES], epoch: i64, reader_pub: &[u8; 32]) -> Vec<u8> {
    let eph = EphemeralSecret::random_from_rng(OsRng);
    let eph_pub = PublicKey::from(&eph);
    let shared = eph.diffie_hellman(&PublicKey::from(*reader_pub));

    let mut context = Vec::with_capacity(40);
    context.extend_from_slice(&epoch.to_be_bytes());
    context.extend_from_slice(reader_pub);
    let wrapping = derive(shared.as_bytes(), WRAP_INFO, &context);

    let cipher = ChaCha20Poly1305::new(Key::from_slice(&wrapping));
    let mut nonce_bytes = [0u8; NONCE_BYTES];
    OsRng.fill_bytes(&mut nonce_bytes);
    let ct = cipher
        .encrypt(
            Nonce::from_slice(&nonce_bytes),
            Payload { msg: key, aad: reader_pub },
        )
        .expect("encryption does not fail for a valid key and nonce");

    let mut out = Vec::with_capacity(32 + NONCE_BYTES + ct.len());
    out.extend_from_slice(eph_pub.as_bytes());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    out
}

/// Recover an epoch key from a wrap addressed to this reader.
pub fn unwrap(
    wrapped: &[u8],
    epoch: i64,
    reader_secret: &StaticSecret,
) -> Result<[u8; KEY_BYTES], SealError> {
    if wrapped.len() < 32 + NONCE_BYTES {
        return Err(SealError::Malformed);
    }
    let (eph_raw, rest) = wrapped.split_at(32);
    let (nonce_bytes, ct) = rest.split_at(NONCE_BYTES);

    let mut eph = [0u8; 32];
    eph.copy_from_slice(eph_raw);
    let shared = reader_secret.diffie_hellman(&PublicKey::from(eph));

    let reader_pub = PublicKey::from(reader_secret).to_bytes();
    let mut context = Vec::with_capacity(40);
    context.extend_from_slice(&epoch.to_be_bytes());
    context.extend_from_slice(&reader_pub);
    let wrapping = derive(shared.as_bytes(), WRAP_INFO, &context);

    let cipher = ChaCha20Poly1305::new(Key::from_slice(&wrapping));
    let plain = cipher
        .decrypt(
            Nonce::from_slice(nonce_bytes),
            Payload { msg: ct, aad: &reader_pub },
        )
        .map_err(|_| SealError::Undecryptable)?;

    if plain.len() != KEY_BYTES {
        return Err(SealError::Malformed);
    }
    let mut key = [0u8; KEY_BYTES];
    key.copy_from_slice(&plain);
    Ok(key)
}
