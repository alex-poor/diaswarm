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
//! diaswarm:3:<subject-hex>:<endpoint-hex>:<purpose>:<relay>:<keys-hex>:<check>
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
//! * **keys** is everything [D26](decisions.md)'s vault needs and the old one
//!   does not: the **Ed25519 key that authors its logs**, and the
//!   `p2panda-encryption` **key bundle** a grant is agreed against. Both,
//!   because neither implies the other and the `subject` field above is a third
//!   key again — `diaswarm-core`'s `Identity` keeps signing and encryption keys
//!   separate on purpose, saying "deriving one from the other is possible and
//!   is the kind of cleverness a review exists to object to". Without the
//!   signer a follower cannot even find the log to fetch; without the bundle it
//!   cannot be granted. About 190 bytes, which a QR code does not notice.
//!   **Empty, and the invite is emitted as v2**, so a subject with no keys
//!   vault still hands out something every existing build understands;
//!   * **check** is four bytes of SHA-256 over everything before it, because the
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
/// The newest version this build *emits*, which is only reached when there is a
/// bundle to carry. See [`Invite::encode`].
const VERSION: &str = "3";
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
    /// The subject's `diaswarm-keys` identity, hex. Empty on a v1 or v2 invite.
    ///
    /// **ITS PRESENCE IS WHAT MAKES AN INVITE v3**, so this is not a field that
    /// can be set carelessly: filling it in makes the invite unreadable to
    /// every build that predates it. That is the intended behaviour — a newer
    /// invite in an older build says so rather than half-working — but it means
    /// a subject only publishes one once it actually has a keys vault to grant
    /// against.
    pub keys: String,
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

/// A keys identity is hex of a CBOR structure, so its length is not fixed —
/// only its alphabet and its parity. Decoding it properly is `diaswarm-keys`'
/// job; this crate must not depend on that one, and a malformed one fails at
/// key agreement with a message about key agreement.
fn keys_ok(b: &str) -> bool {
    !b.is_empty()
        && b.len() % 2 == 0
        && b.len() <= 4096
        && b.bytes().all(|c| c.is_ascii_hexdigit())
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
            keys: String::new(),
        })
    }

    /// The same invite, carrying the subject's keys identity.
    ///
    /// Separate from [`Invite::new_via`] rather than a parameter on it, because
    /// every existing caller wants the invite it already got and adding an
    /// argument would silently make all of them emit v3.
    pub fn with_keys(mut self, keys: &str) -> Result<Self, VaultError> {
        let keys = keys.trim().to_ascii_lowercase();
        if !keys.is_empty() && !keys_ok(&keys) {
            return Err(VaultError::Malformed(
                "a keys identity is an even number of hex characters".into(),
            ));
        }
        self.keys = keys;
        Ok(self)
    }

    /// The string that goes in a QR code, or a message.
    ///
    /// **v2 UNLESS THERE IS A BUNDLE, AND THAT IS THE WHOLE MIGRATION STORY.**
    /// Emitting v3 unconditionally would make every invite unreadable to every
    /// build already installed, for a field most of them have no use for. A
    /// subject that has a keys vault emits v3 and says so; one that does not
    /// emits exactly what it emitted before.
    pub fn encode(&self) -> String {
        let body = if self.keys.is_empty() {
            format!(
                "{PREFIX}:2:{}:{}:{}:{}",
                self.subject,
                self.endpoint,
                self.purpose,
                esc(&self.relay)
            )
        } else {
            format!(
                "{PREFIX}:{VERSION}:{}:{}:{}:{}:{}",
                self.subject,
                self.endpoint,
                self.purpose,
                esc(&self.relay),
                self.keys
            )
        };
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
            // v3 is v2 plus the subject's keys bundle.
            Some("3") => (
                unesc(&parts.get(5).copied().unwrap_or_default().to_ascii_lowercase()),
                8usize,
                "3",
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
            // **THE SHAPE IT NAMES IS THE ONE IT WAS ASKED FOR.** This used to
            // print the v2 form whatever version was being parsed, so a v3
            // invite with a field missing was told to look like a v2 invite —
            // advice that would have made it wrong in a second way.
            let shape = match shown {
                "1" => "diaswarm:1:<subject>:<endpoint>:<purpose>:<check>",
                "3" => "diaswarm:3:<subject>:<endpoint>:<purpose>:<relay>:<bundle>:<check>",
                _ => "diaswarm:2:<subject>:<endpoint>:<purpose>:<relay>:<check>",
            };
            return Err(VaultError::Malformed(format!("not an invite — expected {shape}")));
        }
        let mut invite =
            Invite::new_via(parts[2], parts[3], &parts[4].to_ascii_lowercase(), &relay)?;
        if shown == "3" {
            invite = invite.with_keys(parts[6])?;
        }

        // CHECK LAST, so the specific complaints above are what a person sees.
        // A bad checksum can only say "something is wrong", which is the least
        // useful true statement available.
        //
        // Rebuilt from the ORIGINAL version and the ORIGINAL escaping, not from
        // what this build would emit: a v1 invite re-encoded as v2 would fail
        // its own checksum, which is the kind of error nobody could act on.
        let body = if shown == "1" {
            format!("{PREFIX}:1:{}:{}:{}", invite.subject, invite.endpoint, invite.purpose)
        } else if shown == "3" {
            format!(
                "{PREFIX}:3:{}:{}:{}:{}:{}",
                invite.subject,
                invite.endpoint,
                invite.purpose,
                parts[5].to_ascii_lowercase(),
                invite.keys
            )
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
        // 4 rather than 3: 3 is a format this build emits now, so using it
        // here stopped testing the future and started testing the present.
        let e = Invite::parse(&format!("diaswarm:4:{S}:{E}:follow:x:00000000")).unwrap_err();
        let msg = format!("{e:?}");
        assert!(msg.contains("version 4"), "unhelpful message: {msg}");
    }

    /// A v3 INVITE CARRIES THE BUNDLE AND SURVIVES BEING SHOUTED.
    ///
    /// The checksum covers the bundle, so a transposed character in 300 hex
    /// digits is an error message rather than a key agreement that fails much
    /// later for no visible reason.
    #[test]
    fn a_bundle_survives_the_round_trip_and_the_checksum_covers_it() {
        let bundle = "a36c6964656e746974795f6b6579582000112233445566778899aabbccddeeff";
        let invite = Invite::new(S, E, "follow").unwrap().with_keys(bundle).unwrap();
        let text = invite.encode();
        assert!(text.starts_with("diaswarm:3:"), "a bundle should make it v3: {text}");

        let back = Invite::parse(&text).unwrap();
        assert_eq!(back.keys, bundle);
        assert_eq!(back, invite);

        // Shouted by a chat client, and still the same invite.
        assert_eq!(Invite::parse(&text.to_ascii_uppercase()).unwrap(), invite);

        // One wrong character anywhere in the bundle is caught.
        let mut broken: Vec<char> = text.chars().collect();
        let at = text.find(bundle).unwrap() + 10;
        broken[at] = if broken[at] == 'a' { 'b' } else { 'a' };
        let broken: String = broken.into_iter().collect();
        assert!(Invite::parse(&broken).is_err(), "a damaged bundle was accepted");
    }

    /// **A SUBJECT WITH NO KEYS VAULT STILL EMITS WHAT EVERY BUILD UNDERSTANDS.**
    ///
    /// The version is a consequence of having a bundle, not a build flag. If
    /// this emitted v3 unconditionally, every invite from an updated phone
    /// would be unreadable to every phone that had not updated — for a field
    /// those phones have no use for.
    #[test]
    fn no_bundle_means_a_version_2_invite() {
        let invite = Invite::new(S, E, "follow").unwrap();
        let text = invite.encode();
        assert!(text.starts_with("diaswarm:2:"), "expected v2 without a bundle: {text}");
        assert_eq!(Invite::parse(&text).unwrap(), invite);

        // And an empty bundle is the same as no bundle, not a v3 with a hole.
        let explicit = Invite::new(S, E, "follow").unwrap().with_keys("").unwrap();
        assert_eq!(explicit.encode(), text);

        // Rubbish in the bundle is refused at the point it is set.
        assert!(Invite::new(S, E, "follow").unwrap().with_keys("zz").is_err());
        assert!(Invite::new(S, E, "follow").unwrap().with_keys("abc").is_err());
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
