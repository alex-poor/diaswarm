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
//! diaswarm:2:<subject-hex>:<endpoint-hex>:<purpose>:<relay>:<check>
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
//! * **relay** is which rendezvous server this subject can be reached through
//!   (D23). CARRIED RATHER THAN COMPILED IN, because a relay baked into the
//!   binary is one every phone must be rebuilt to change — and the whole point
//!   of being able to run your own is that switching to it should cost an
//!   invite, not a release. Empty means "direct only": findable on the local
//!   network and nowhere else. `:` is written `%3A` and `%` as `%25`, since
//!   the fields are colon-separated; `/` needs no escaping and stays readable;
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
const VERSION: &str = "2";
pub const DEFAULT_PURPOSE: &str = "follow";

/// Where a peer is reachable when it is not on your wifi.
///
/// **A DEFAULT, NOT A DEPENDENCY.** `n0` runs these publicly and without
/// accounts, which is what lets the app work when it is installed rather than
/// after somebody stands up a server. It is also a third party who can see
/// which node ids exchange bytes, from which addresses and how often — never
/// what they say, which is sealed twice over, but that metadata is real (D23).
/// Anyone who would rather not hand it over runs `iroh-relay` and puts their
/// own URL in the invite; nothing else changes, and no phone is rebuilt.
///
/// Asia-Pacific because that is where these phones are. Verified 2026-09-11:
/// `aps1-1.relay.n0.iroh.link` resolves into `2a01:4ff:2f0::/48`, registered to
/// Hetzner Online GmbH, netname CLOUD-SIN, country SG.
pub const DEFAULT_RELAY: &str = "https://aps1-1.relay.n0.iroh.link.";

/// Colons separate the fields, so a URL's own colons have to go.
///
/// Lowercase hex on purpose: the whole invite is compared case-insensitively,
/// so an escape written `%3A` and normalised to `%3a` would checksum
/// differently from the one that was encoded. Emitting the normalised form
/// makes that normalisation a no-op instead of a bug.
fn esc(s: &str) -> String {
    s.replace('%', "%25").replace(':', "%3a")
}

fn unesc(s: &str) -> String {
    // `%3A` before `%25`: doing it the other way round would turn a literal
    // "%253A" into a colon, which is the classic double-decoding bug.
    s.replace("%3A", ":").replace("%3a", ":").replace("%25", "%")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    pub subject: String,
    pub endpoint: String,
    pub purpose: String,
    /// Empty means direct connections only — the local network and nothing else.
    pub relay: String,
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
    /// An invite reachable through the default relay.
    pub fn new(subject: &str, endpoint: &str, purpose: &str) -> Result<Self, VaultError> {
        Self::new_via(subject, endpoint, purpose, DEFAULT_RELAY)
    }

    /// An invite naming the relay this subject can be reached through. Pass an
    /// empty relay for a subject that should only ever be found directly.
    pub fn new_via(
        subject: &str,
        endpoint: &str,
        purpose: &str,
        relay: &str,
    ) -> Result<Self, VaultError> {
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
        // LOWERCASED, TO KEEP THE FORMAT SHOUTABLE. Everything else in an
        // invite is hex or a restricted purpose, so a chat client or a scanner
        // upper-casing the lot changed nothing. A URL is the first field where
        // case could matter — and a relay URL is scheme, host and port, all of
        // which are case-insensitive by definition. A relay behind a
        // case-sensitive PATH is the one setup this would break, and it breaks
        // loudly at connect time rather than silently.
        let relay = relay.trim().to_ascii_lowercase();
        if relay.contains(char::is_whitespace) {
            return Err(VaultError::Malformed("a relay url has no spaces in it".into()));
        }
        Ok(Invite {
            subject,
            endpoint,
            purpose: purpose.to_string(),
            relay,
        })
    }

    /// The string that goes in a QR code, or a message.
    pub fn encode(&self) -> String {
        let body = format!(
            "{PREFIX}:{VERSION}:{}:{}:{}:{}",
            self.subject,
            self.endpoint,
            self.purpose,
            esc(&self.relay)
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
        if !parts.first().is_some_and(|p| p.eq_ignore_ascii_case(PREFIX)) {
            return Err(VaultError::Malformed("not a diaswarm invite".into()));
        }
        // VERSION 1 IS STILL READ, and this is not politeness. Invites are
        // scanned off screens and pasted into messages; the ones already handed
        // out do not stop existing because the format grew a field. A v1 invite
        // predates relays entirely, so it means what it meant when it was
        // written: reachable however this build reaches people by default.
        let (relay, want, shown) = match parts.get(1).copied() {
            Some("1") => (DEFAULT_RELAY.to_string(), 6usize, "1"),
            Some("2") => (
                unesc(&parts.get(5).copied().unwrap_or_default().to_ascii_lowercase()),
                7usize,
                "2",
            ),
            Some(v) => {
                // Named explicitly: a newer invite pasted into an older build
                // must say so, not fail as though the person mistyped it.
                return Err(VaultError::Malformed(format!(
                    "invite version {v} — this build understands {VERSION}. Update to use it."
                )));
            }
            None => return Err(VaultError::Malformed("not a diaswarm invite".into())),
        };
        if parts.len() != want {
            return Err(VaultError::Malformed(
                "not an invite — expected                  diaswarm:2:<subject>:<endpoint>:<purpose>:<relay>:<check>"
                    .into(),
            ));
        }
        let invite =
            Invite::new_via(parts[2], parts[3], &parts[4].to_ascii_lowercase(), &relay)?;

        // CHECK LAST, so the specific complaints above are what a person sees.
        // A bad checksum can only say "something is wrong", which is the least
        // useful true statement available.
        //
        // Rebuilt from the ORIGINAL version and the ORIGINAL escaping, not from
        // what this build would emit: a v1 invite re-encoded as v2 would fail
        // its own checksum, which is the kind of error nobody could act on.
        let body = if shown == "1" {
            format!("{PREFIX}:1:{}:{}:{}", invite.subject, invite.endpoint, invite.purpose)
        } else {
            format!(
                "{PREFIX}:2:{}:{}:{}:{}",
                invite.subject,
                invite.endpoint,
                invite.purpose,
                // The field as normalised, not as typed, so the same invite
                // shouted by a chat client still checks out.
                parts[5].to_ascii_lowercase()
            )
        };
        if !parts[want - 1].eq_ignore_ascii_case(&check_of(&body)) {
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
        let e = Invite::parse(&format!("diaswarm:3:{S}:{E}:follow:x:00000000")).unwrap_err();
        let msg = format!("{e:?}");
        assert!(msg.contains("version 3"), "unhelpful message: {msg}");
    }

    /// **INVITES ALREADY HANDED OUT DO NOT STOP EXISTING** because the format
    /// grew a field. One is on a phone in this house right now.
    #[test]
    fn a_version_1_invite_still_works_and_means_the_default_relay() {
        let v1 = {
            let body = format!("diaswarm:1:{S}:{E}:follow");
            format!("{body}:{}", check_of(&body))
        };
        let got = Invite::parse(&v1).expect("a v1 invite was refused");
        assert_eq!(got.subject, S);
        assert_eq!(got.endpoint, E);
        assert_eq!(got.relay, DEFAULT_RELAY, "v1 should mean the default relay");
    }

    #[test]
    fn a_relay_survives_the_round_trip_colons_and_all() {
        for relay in [
            DEFAULT_RELAY,
            "https://relay.example.org:8443/",
            "http://10.0.0.9:3340",
            "", // direct only
        ] {
            let i = Invite::new_via(S, E, "follow", relay).unwrap();
            let encoded = i.encode();
            assert_eq!(
                encoded.split(':').count(),
                7,
                "a relay's own colons broke the field count: {encoded}"
            );
            assert_eq!(Invite::parse(&encoded).unwrap(), i, "relay {relay:?} did not survive");
            assert_eq!(Invite::parse(&encoded).unwrap().relay, relay.to_ascii_lowercase());
        }
    }

    /// The checksum exists so a typo is an error rather than a silence, and the
    /// relay is now the field most likely to be retyped by hand.
    #[test]
    fn a_damaged_relay_is_refused_rather_than_dialled() {
        let good = Invite::new_via(S, E, "follow", "https://relay.example.org").unwrap().encode();
        let bad = good.replace("relay.example.org", "relay.exampl3.org");
        assert_ne!(bad, good);
        assert!(Invite::parse(&bad).is_err(), "an invite with a corrupted relay was accepted");
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
