//! SMART Health Links: a standards-based route to an EHR that needs no server
//! from us.
//!
//! **WHY THIS ONE AND NOT THE OTHER TWO.** The CGM IG offers three ways to get
//! data to a clinician. `$submit-cgm-bundle` needs the clinic to run a FHIR
//! server implementing that operation; the PDF report is delivered inside the
//! same submission and inherits the requirement. **A SMART Health Link needs
//! only a receiver on the clinic side — no FHIR server** — which is the
//! difference between a pathway that exists on paper and one a real clinic
//! could use.
//!
//! 🔑 **AND THE TRUST MODEL IS THIS PROJECT'S OWN.** The link carries a 32-byte
//! key in its payload; the host stores AES-256-GCM ciphertext and a filename.
//! The spec's word for the host is a **"blind intermediary"**. That is exactly
//! `feasibility.md` §9.2's holder-that-cannot-read, arriving from the other
//! direction — so putting a document behind a SHL does not create the custodian
//! that uploading to Nightscout would.
//!
//! ⚠️ **THIS MODULE HOSTS NOTHING**, which keeps [D33](../../../docs/decisions.md)
//! intact. It produces two artefacts — an encrypted file and a link — and where
//! the file goes is the operator's choice. The `U` flag says the URL resolves
//! to a single encrypted file fetched with a plain GET, so **any static host
//! works**: no manifest endpoint, no POST handler, no application.
//!
//! **What it deliberately does not do:** mint a viewer URL. A `shlink:` is
//! complete on its own; wrapping it in `https://some-viewer.example#shlink:/…`
//! names a third party as the place a clinician's browser goes, and that choice
//! is not this tool's to make.

use aes_gcm::aead::{Aead, KeyInit, OsRng, Payload, rand_core::RngCore};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;

/// A link and the file it points at.
pub struct Shl {
    /// `shlink:/…` — hand this to the clinician.
    pub link: String,
    /// The JWE compact serialization to publish at `url`.
    pub encrypted: String,
    /// The key, base64url, as it appears inside the link.
    pub key_b64: String,
}

/// Wrap a FHIR document as a SMART Health Link.
///
/// `url` is where the caller will publish [`Shl::encrypted`]. It goes into the
/// link verbatim, so it must be the URL a receiver can GET — not a directory.
///
/// ⚠️ **`exp` IS A COURTESY, NOT AN ACCESS CONTROL.** A receiver is asked to
/// stop honouring the link after it; nothing enforces it, and anybody who
/// fetched the ciphertext and read the key keeps both. That is the same
/// property [§7.4](../../../docs/feasibility.md) states about the swarm, and a
/// UI must not describe it as a revocation.
pub fn wrap(document: &str, url: &str, label: &str, exp: Option<u64>) -> Shl {
    // 32 random bytes, as the specification requires.
    let mut key_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut key_bytes);
    let key_b64 = B64.encode(key_bytes);

    // **JWE COMPACT, alg=dir, enc=A256GCM**, which is what a SHL receiver
    // expects and the only thing it will try. `cty` tells it what it decrypted.
    let header = r#"{"alg":"dir","enc":"A256GCM","cty":"application/fhir+json"}"#;
    let header_b64 = B64.encode(header);

    let mut iv = [0u8; 12];
    OsRng.fill_bytes(&mut iv);

    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key_bytes));
    // **THE PROTECTED HEADER IS THE AAD**, as ASCII of its base64url form. Get
    // this wrong and every receiver reports a corrupt file rather than a
    // mismatch, which is a miserable thing to debug.
    let sealed = cipher
        .encrypt(
            Nonce::from_slice(&iv),
            Payload { msg: document.as_bytes(), aad: header_b64.as_bytes() },
        )
        .expect("AES-GCM encryption cannot fail with a valid key and nonce");

    // GCM appends the 16-byte tag; JWE carries it as its own segment.
    let (ciphertext, tag) = sealed.split_at(sealed.len() - 16);

    // `hdr..iv.ct.tag` — the empty segment is the encrypted key, which `dir`
    // does not use.
    let encrypted = format!(
        "{header_b64}..{}.{}.{}",
        B64.encode(iv),
        B64.encode(ciphertext),
        B64.encode(tag)
    );

    let mut payload = format!(
        r#"{{"url":"{}","key":"{key_b64}","flag":"U","label":"{}""#,
        escape(url),
        escape(label)
    );
    if let Some(exp) = exp {
        payload.push_str(&format!(r#","exp":{exp}"#));
    }
    payload.push('}');

    Shl { link: format!("shlink:/{}", B64.encode(&payload)), encrypted, key_b64 }
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value as J;

    fn decode_link(link: &str) -> J {
        let b64 = link.strip_prefix("shlink:/").expect("not a shlink");
        serde_json::from_slice(&B64.decode(b64).expect("payload is not base64url")).unwrap()
    }

    /// THE LINK IS WHAT THE SPECIFICATION SAYS IT IS.
    #[test]
    fn the_payload_carries_the_fields_a_receiver_reads() {
        let shl = wrap("{}", "https://files.example/abc", "CGM summary", Some(1_800_000_000));
        let p = decode_link(&shl.link);
        assert_eq!(p["url"], "https://files.example/abc");
        assert_eq!(p["key"], shl.key_b64);
        assert_eq!(p["flag"], "U", "without U a receiver POSTs for a manifest we do not serve");
        assert_eq!(p["label"], "CGM summary");
        assert_eq!(p["exp"], 1_800_000_000i64);
        // 32 bytes, base64url, no padding.
        assert_eq!(B64.decode(p["key"].as_str().unwrap()).unwrap().len(), 32);
    }

    /// AND THE FILE ACTUALLY DECRYPTS WITH THE KEY IN THE LINK.
    ///
    /// **THE TEST THAT MATTERS.** Everything else is shape; this is whether a
    /// clinician's receiver gets the document back. It reproduces what a
    /// receiver does: split the compact form, take the key from the payload,
    /// use the protected header as AAD.
    #[test]
    fn a_receiver_can_decrypt_it_with_only_the_link() {
        let doc = r#"{"resourceType":"Bundle","type":"document"}"#;
        let shl = wrap(doc, "https://files.example/abc", "CGM summary", None);
        let p = decode_link(&shl.link);

        let key = B64.decode(p["key"].as_str().unwrap()).unwrap();
        let parts: Vec<&str> = shl.encrypted.split('.').collect();
        assert_eq!(parts.len(), 5, "not a JWE compact serialization");
        assert!(parts[1].is_empty(), "alg=dir must leave the encrypted key empty");

        let header: J = serde_json::from_slice(&B64.decode(parts[0]).unwrap()).unwrap();
        assert_eq!(header["alg"], "dir");
        assert_eq!(header["enc"], "A256GCM");
        assert_eq!(header["cty"], "application/fhir+json");

        let iv = B64.decode(parts[2]).unwrap();
        let mut blob = B64.decode(parts[3]).unwrap();
        blob.extend_from_slice(&B64.decode(parts[4]).unwrap());

        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        let out = cipher
            .decrypt(
                Nonce::from_slice(&iv),
                Payload { msg: &blob, aad: parts[0].as_bytes() },
            )
            .expect("a receiver could not decrypt the file");
        assert_eq!(String::from_utf8(out).unwrap(), doc);
    }

    /// A DIFFERENT LINK MUST NOT OPEN THIS FILE.
    #[test]
    fn another_links_key_does_not_work() {
        let a = wrap("secret", "https://x/1", "a", None);
        let b = wrap("other", "https://x/2", "b", None);
        assert_ne!(a.key_b64, b.key_b64, "two wraps produced the same key");

        let key = B64.decode(&b.key_b64).unwrap();
        let parts: Vec<&str> = a.encrypted.split('.').collect();
        let iv = B64.decode(parts[2]).unwrap();
        let mut blob = B64.decode(parts[3]).unwrap();
        blob.extend_from_slice(&B64.decode(parts[4]).unwrap());
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        assert!(
            cipher
                .decrypt(Nonce::from_slice(&iv), Payload { msg: &blob, aad: parts[0].as_bytes() })
                .is_err(),
            "the wrong key opened the file"
        );
    }

    /// A TAMPERED HEADER MUST FAIL, WHICH IS WHAT THE AAD IS FOR.
    #[test]
    fn the_header_is_authenticated() {
        let shl = wrap("payload", "https://x/1", "a", None);
        let p = decode_link(&shl.link);
        let key = B64.decode(p["key"].as_str().unwrap()).unwrap();
        let parts: Vec<&str> = shl.encrypted.split('.').collect();
        let iv = B64.decode(parts[2]).unwrap();
        let mut blob = B64.decode(parts[3]).unwrap();
        blob.extend_from_slice(&B64.decode(parts[4]).unwrap());
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        let forged = B64.encode(r#"{"alg":"dir","enc":"A256GCM","cty":"text/plain"}"#);
        assert!(
            cipher
                .decrypt(Nonce::from_slice(&iv), Payload { msg: &blob, aad: forged.as_bytes() })
                .is_err(),
            "the protected header was not authenticated"
        );
    }
}
