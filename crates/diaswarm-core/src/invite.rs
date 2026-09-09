//! An invite: everything one person needs to start following another.
//!
//! WHY THIS IS NOT A CONVENIENCE. Sharing needed two 64-character hex strings
//! moved between two devices by hand — the subject's key and the endpoint to
//! dial. One transposed character produces a key that is perfectly valid and
//! belongs to nobody, and the resulting failure is silence: the fetch reaches
//! no one, or reaches someone with nothing to say. A format that carries both
//! together, with a checksum over them, turns a typo into an error message.
//!
//! It also decides what "share my data" means at the human end. The answer has
//! to be one thing you can hold up to a phone, not a procedure.
//!
//! ```text
//! diaswarm:1:<subject-hex>:<endpoint-hex>:<purpose>:<check>
//! ```
//!
//! * **version** first, so a future format is refused clearly rather than
//!   parsed into something wrong;
//! * **subject** is the X25519 public key: what data is being offered, and what
//!   a grant is made against;
//! * **endpoint** is where to start looking. NOT where the data must come from
//!   — any peer holding the subject serves identical bytes (D15), and a
//!   follower accumulates more endpoints as it finds them. This is a first
//!   contact, not an address of record;
//! * **purpose** scopes the wrap tag, so the same reader granted twice for
//!   different reasons holds two unrelated tags (D13);
//! * **check** is four bytes of SHA-256 over everything before it, because the
//!   failure this format exists to prevent is a silent one.
//!
//! WHAT AN INVITE IS NOT: a secret, and not a grant. Anyone holding it can
//! fetch the ciphertext and open none of it. Access still requires the subject
//! to grant that specific key, which is a separate act on the subject's device.
//! An invite leaked to a stranger costs the subject the same as publishing
//! their public key, which is to say nothing.

use sha2::{Digest, Sha256};

use crate::vault::{hex, unhex, VaultError};

const PREFIX: &str = "diaswarm";
const VERSION: &str = "1";
pub const DEFAULT_PURPOSE: &str = "follow";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    pub subject: String,
    pub endpoint: String,
    pub purpose: String,
}

fn check_of(body: &str) -> String {
    hex(&Sha256::digest(body.as_bytes())[..4])
}

/// A purpose has to survive being a path component and a field separator.
///
/// Checked here rather than at the point it becomes a filename, because by
/// then it is inside a tag and unrecognisable.
fn purpose_ok(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 64
        && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn key_ok(k: &str) -> bool {
    k.len() == 64 && k.bytes().all(|b| b.is_ascii_hexdigit())
}

impl Invite {
    pub fn new(subject: &str, endpoint: &str, purpose: &str) -> Result<Self, VaultError> {
        let subject = subject.trim().to_ascii_lowercase();
        let endpoint = endpoint.trim().to_ascii_lowercase();
        if !key_ok(&subject) {
            return Err(VaultError::Malformed("a subject is 64 hex characters".into()));
        }
        if !key_ok(&endpoint) {
            return Err(VaultError::Malformed("an endpoint id is 64 hex characters".into()));
        }
        if !purpose_ok(purpose) {
            return Err(VaultError::Malformed(
                "a purpose is 1-64 characters of letters, digits, - or _".into(),
            ));
        }
        Ok(Invite { subject, endpoint, purpose: purpose.to_string() })
    }

    /// The string that goes in a QR code, or a message.
    pub fn encode(&self) -> String {
        let body = format!(
            "{PREFIX}:{VERSION}:{}:{}:{}",
            self.subject, self.endpoint, self.purpose
        );
        let check = check_of(&body);
        format!("{body}:{check}")
    }

    /// Read one back, refusing anything it cannot vouch for.
    pub fn parse(s: &str) -> Result<Self, VaultError> {
        let s = s.trim();
        // Tolerate a scanner or a chat client upper-casing the whole thing:
        // the payload is hex and a restricted purpose, so case carries nothing.
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 6 {
            return Err(VaultError::Malformed(
                "not an invite — expected diaswarm:1:<subject>:<endpoint>:<purpose>:<check>".into(),
            ));
        }
        if !parts[0].eq_ignore_ascii_case(PREFIX) {
            return Err(VaultError::Malformed("not a diaswarm invite".into()));
        }
        if parts[1] != VERSION {
            // Named explicitly: a newer invite pasted into an older build must
            // say so, not fail as though the person mistyped it.
            return Err(VaultError::Malformed(format!(
                "invite version {} — this build understands {VERSION}. Update to use it.",
                parts[1]
            )));
        }
        let invite = Invite::new(parts[2], parts[3], &parts[4].to_ascii_lowercase())?;

        // CHECK LAST, so the specific complaints above are what a person sees.
        // A bad checksum can only say "something is wrong", which is the least
        // useful true statement available.
        let body = format!("{PREFIX}:{VERSION}:{}:{}:{}", invite.subject, invite.endpoint, invite.purpose);
        if !parts[5].eq_ignore_ascii_case(&check_of(&body)) {
            return Err(VaultError::Malformed(
                "this invite is damaged — a character is wrong somewhere. Ask for it again."
                    .into(),
            ));
        }
        Ok(invite)
    }

    /// The subject key as bytes, for granting against.
    pub fn subject_bytes(&self) -> Result<[u8; 32], VaultError> {
        unhex(&self.subject)?
            .try_into()
            .map_err(|_| VaultError::Malformed("subject key is not 32 bytes".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // OBVIOUSLY NOT REAL KEYS. These were copied from a live phone while the
    // format was being worked out, which put a real subject key and a real
    // endpoint id — together, a working invite to somebody's actual vault —
    // into a repository that was always going to be public. The data is
    // ciphertext and unreadable, but the address is not: it says whose device
    // this is and when it is online.
    const S: &str = "5111111111111111111111111111111111111111111111111111111111111111";
    const E: &str = "9222222222222222222222222222222222222222222222222222222222222222";

    #[test]
    fn round_trips() {
        let i = Invite::new(S, E, "follow").unwrap();
        assert_eq!(Invite::parse(&i.encode()).unwrap(), i);
    }

    #[test]
    fn a_single_wrong_character_is_refused() {
        // The whole reason for the checksum. Swapping one hex digit yields a
        // key that is structurally perfect and belongs to nobody, and the
        // resulting failure would otherwise be silence.
        let good = Invite::new(S, E, "follow").unwrap().encode();
        let mut bad: Vec<char> = good.chars().collect();
        let at = good.find(S).unwrap() + 5;
        bad[at] = if bad[at] == 'a' { 'b' } else { 'a' };
        let bad: String = bad.into_iter().collect();
        assert_ne!(bad, good);
        assert!(Invite::parse(&bad).is_err(), "a corrupted invite was accepted");
    }

    #[test]
    fn case_and_whitespace_survive_a_round_trip_through_a_human() {
        let i = Invite::new(S, E, "follow").unwrap();
        let shouted = format!("  {}  ", i.encode().to_ascii_uppercase());
        assert_eq!(Invite::parse(&shouted).unwrap(), i);
    }

    #[test]
    fn a_future_version_says_so_rather_than_looking_like_a_typo() {
        let e = Invite::parse(&format!("diaswarm:2:{S}:{E}:follow:00000000")).unwrap_err();
        let msg = format!("{e:?}");
        assert!(msg.contains("version 2"), "unhelpful message: {msg}");
    }

    #[test]
    fn a_purpose_cannot_smuggle_a_separator_or_a_path() {
        for bad in ["", "a:b", "../../etc", "with space", &"x".repeat(65)] {
            assert!(Invite::new(S, E, bad).is_err(), "accepted purpose {bad:?}");
        }
    }

    #[test]
    fn keys_must_be_the_right_shape() {
        assert!(Invite::new("short", E, "follow").is_err());
        assert!(Invite::new(S, "nothex_nothex_nothex_nothex_nothex_nothex_nothex_nothex_nothexZZ", "follow").is_err());
    }
}
