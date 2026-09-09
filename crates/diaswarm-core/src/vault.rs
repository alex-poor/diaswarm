//! A vault: one subject's sealed history, the wraps that open it, and the
//! signed record of who was granted what.
//!
//! Files, not a swarm. Where these bytes go, how they replicate and who holds
//! them is a later stage, and nothing here has an opinion — which is the point.
//! A holder can copy a whole vault and learn nothing but its shape.
//!
//! ```text
//! vault/
//!   meta.json                     spec version, epoch basis, subject public key
//!   epochs/<epoch>.seal           records for that day, sealed under its key
//!   wraps/<epoch>/<reader>.wrap   that key, wrapped to one reader
//!   grants.ndjson                 signed: who was granted what, and when it stopped
//! ```
//!
//! **Revocation is the absence of a file.** There is no delete step and no
//! message to anyone: a revoked reader is one no new wrap is written for. The
//! wraps they already fetched keep working, because nothing here can reach into
//! a copy someone already has — and claiming otherwise is the recall
//! feasibility.md §11 forbids implying.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use x25519_dalek::{PublicKey, StaticSecret};

use crate::seal::{epoch_key, open_epoch, seal_epoch, unwrap, wrap, KEY_BYTES};
use crate::{encode, epoch_of, Record, SPEC_VERSION};

#[derive(Debug)]
pub enum VaultError {
    Io(std::io::Error),
    Malformed(String),
    /// The vault was written by a spec version this build does not implement.
    SpecMismatch { found: u64, expected: u64 },
}

impl From<std::io::Error> for VaultError {
    fn from(e: std::io::Error) -> Self {
        VaultError::Io(e)
    }
}

/// One party's keys: a signing key for grants, an encryption key for wraps.
///
/// Two keys rather than one because they are used by different parties for
/// different things — the subject signs grants everyone verifies, a reader
/// decrypts wraps only they can open. Deriving one from the other is possible
/// and is the kind of cleverness a review exists to object to.
pub struct Identity {
    pub signing: SigningKey,
    pub encryption: StaticSecret,
}

impl Identity {
    pub fn generate() -> Self {
        Identity {
            signing: SigningKey::generate(&mut rand_core::OsRng),
            encryption: StaticSecret::random_from_rng(rand_core::OsRng),
        }
    }

    pub fn enc_public(&self) -> [u8; 32] {
        PublicKey::from(&self.encryption).to_bytes()
    }

    pub fn verifying(&self) -> VerifyingKey {
        self.signing.verifying_key()
    }

    /// 64 bytes: signing seed then encryption scalar.
    pub fn to_bytes(&self) -> [u8; 64] {
        let mut out = [0u8; 64];
        out[..32].copy_from_slice(&self.signing.to_bytes());
        out[32..].copy_from_slice(&self.encryption.to_bytes());
        out
    }

    pub fn from_bytes(b: &[u8; 64]) -> Self {
        let mut sk = [0u8; 32];
        sk.copy_from_slice(&b[..32]);
        let mut ek = [0u8; 32];
        ek.copy_from_slice(&b[32..]);
        Identity {
            signing: SigningKey::from_bytes(&sk),
            encryption: StaticSecret::from(ek),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Meta {
    spec: u64,
    epoch: String,
    subject: String,
}

/// A signed statement that a grant started or stopped.
///
/// This is the whole of what feasibility.md §11 offers in place of a read log:
/// every grant and every withdrawal, signed, not rewritable by the reader. It is
/// deliberately NOT a record of anyone reading — nothing here can produce one.
///
/// NOTE, and it is §12.0: this record is public and it names the reader. The
/// data is encrypted; the social graph is not. That is the sharpest unsolved
/// problem in the design, and this struct is where it becomes concrete.
#[derive(Clone, Serialize, Deserialize)]
pub struct Grant {
    pub act: String,
    pub epoch: i64,
    pub purpose: String,
    pub reader: String,
    pub subject: String,
    pub sig: String,
}

fn grant_payload(act: &str, epoch: i64, purpose: &str, reader: &str, subject: &str) -> Vec<u8> {
    // Sorted keys, no spaces — the same canonical shape as a record, so what is
    // signed is exactly what is on disk.
    format!(
        r#"{{"act":"{act}","epoch":{epoch},"purpose":"{purpose}","reader":"{reader}","subject":"{subject}"}}"#
    )
    .into_bytes()
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn unhex(s: &str) -> Result<Vec<u8>, VaultError> {
    if s.len() % 2 != 0 {
        return Err(VaultError::Malformed("odd-length hex".into()));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| VaultError::Malformed("hex".into())))
        .collect()
}

pub struct Vault {
    root: PathBuf,
    subject_pub: [u8; 32],
}

impl Vault {
    pub fn create(root: &Path, subject: &Identity) -> Result<Self, VaultError> {
        fs::create_dir_all(root.join("epochs"))?;
        fs::create_dir_all(root.join("wraps"))?;
        let meta = Meta {
            spec: SPEC_VERSION,
            epoch: crate::EPOCH_BASIS.to_string(),
            subject: hex(&subject.enc_public()),
        };
        fs::write(root.join("meta.json"), serde_json::to_vec_pretty(&meta).unwrap())?;
        Ok(Vault { root: root.to_path_buf(), subject_pub: subject.enc_public() })
    }

    pub fn open(root: &Path) -> Result<Self, VaultError> {
        let meta: Meta = serde_json::from_slice(&fs::read(root.join("meta.json"))?)
            .map_err(|e| VaultError::Malformed(e.to_string()))?;
        if meta.spec != SPEC_VERSION {
            return Err(VaultError::SpecMismatch { found: meta.spec, expected: SPEC_VERSION });
        }
        let bytes = unhex(&meta.subject)?;
        let mut subject_pub = [0u8; 32];
        subject_pub.copy_from_slice(&bytes);
        Ok(Vault { root: root.to_path_buf(), subject_pub })
    }

    pub fn subject_pub(&self) -> [u8; 32] {
        self.subject_pub
    }

    /// Seal one epoch's records and keep its key for wrapping.
    ///
    /// The key is written beside the sealed data under `epochs/<n>.key`. That
    /// file is the subject's own copy and must never leave the device: it is
    /// what makes every later grant possible without re-sealing anything.
    pub fn seal(&self, epoch: i64, records: &[Record]) -> Result<(), VaultError> {
        // REUSE THE EPOCH'S KEY IF IT HAS ONE. A day is sealed repeatedly as
        // its records arrive, and minting a fresh key each time would silently
        // invalidate every wrap already published for that epoch — readers
        // would hold a key that opens nothing, with no error anywhere to say
        // so. The key belongs to the epoch, not to the act of sealing.
        let key = match self.epoch_key_of(epoch) {
            Ok(existing) => existing,
            Err(_) => epoch_key(),
        };
        let plaintext = encode(records);
        let sealed = seal_epoch(plaintext.as_bytes(), &key, epoch, &self.subject_pub);
        fs::write(self.root.join("epochs").join(format!("{epoch}.seal")), sealed)?;
        fs::write(self.root.join("epochs").join(format!("{epoch}.key")), key)?;
        Ok(())
    }

    pub fn epochs(&self) -> Result<Vec<i64>, VaultError> {
        let mut out = Vec::new();
        for entry in fs::read_dir(self.root.join("epochs"))? {
            let name = entry?.file_name().to_string_lossy().to_string();
            if let Some(stem) = name.strip_suffix(".seal") {
                if let Ok(n) = stem.parse::<i64>() {
                    out.push(n);
                }
            }
        }
        out.sort_unstable();
        Ok(out)
    }

    fn epoch_key_of(&self, epoch: i64) -> Result<[u8; KEY_BYTES], VaultError> {
        let raw = fs::read(self.root.join("epochs").join(format!("{epoch}.key")))?;
        if raw.len() != KEY_BYTES {
            return Err(VaultError::Malformed("epoch key is the wrong length".into()));
        }
        let mut k = [0u8; KEY_BYTES];
        k.copy_from_slice(&raw);
        Ok(k)
    }

    pub fn record_grant(
        &self,
        subject: &Identity,
        reader_pub: &[u8; 32],
        purpose: &str,
        act: &str,
        epoch: i64,
    ) -> Result<(), VaultError> {
        let reader = hex(reader_pub);
        let subj = hex(&self.subject_pub);
        let payload = grant_payload(act, epoch, purpose, &reader, &subj);
        let sig: Signature = subject.signing.sign(&payload);
        let grant = Grant {
            act: act.into(),
            epoch,
            purpose: purpose.into(),
            reader,
            subject: subj,
            sig: hex(&sig.to_bytes()),
        };
        let mut line = serde_json::to_string(&grant).unwrap();
        line.push('\n');
        use std::io::Write;
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.root.join("grants.ndjson"))?;
        f.write_all(line.as_bytes())?;
        Ok(())
    }

    pub fn grants(&self) -> Result<Vec<Grant>, VaultError> {
        let path = self.root.join("grants.ndjson");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let text = fs::read_to_string(path)?;
        let mut out = Vec::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            out.push(serde_json::from_str(line).map_err(|e| VaultError::Malformed(e.to_string()))?);
        }
        Ok(out)
    }

    /// Verify a grant against the subject's signing key.
    pub fn verify(grant: &Grant, subject: &VerifyingKey) -> bool {
        let payload = grant_payload(
            &grant.act,
            grant.epoch,
            &grant.purpose,
            &grant.reader,
            &grant.subject,
        );
        let Ok(bytes) = unhex(&grant.sig) else { return false };
        let Ok(sig) = Signature::from_slice(&bytes) else { return false };
        subject.verify(&payload, &sig).is_ok()
    }

    /// Which epochs a reader is currently entitled to, from the grant log.
    ///
    /// An epoch is live if the most recent statement at or before it was a
    /// grant. Entitlement is a property of the log, not of anything a server
    /// decides.
    pub fn entitled(&self, reader_pub: &[u8; 32], purpose: &str) -> Result<BTreeSet<i64>, VaultError> {
        let reader = hex(reader_pub);
        let mut mine: Vec<Grant> = self
            .grants()?
            .into_iter()
            .filter(|g| g.reader == reader && g.purpose == purpose)
            .collect();
        mine.sort_by_key(|g| g.epoch);

        let mut live = BTreeSet::new();
        for epoch in self.epochs()? {
            let mut state: Option<&str> = None;
            for g in &mine {
                if g.epoch <= epoch {
                    state = Some(&g.act);
                }
            }
            if state == Some("grant") {
                live.insert(epoch);
            }
        }
        Ok(live)
    }

    /// Wrap every epoch this reader is entitled to and has not already been sent.
    ///
    /// THIS IS REVOCATION, and it is entirely negative space: a stopped reader
    /// is one this loop no longer wraps for. Returns how many new wraps were
    /// written.
    pub fn publish_wraps(&self, reader_pub: &[u8; 32], purpose: &str) -> Result<usize, VaultError> {
        let reader = hex(reader_pub);
        let mut written = 0;
        for epoch in self.entitled(reader_pub, purpose)? {
            let dir = self.root.join("wraps").join(epoch.to_string());
            fs::create_dir_all(&dir)?;
            let path = dir.join(format!("{reader}.wrap"));
            if path.exists() {
                continue; // a wrap is not re-issued
            }
            let key = self.epoch_key_of(epoch)?;
            fs::write(path, wrap(&key, epoch, reader_pub))?;
            written += 1;
        }
        Ok(written)
    }

    /// Everything a reader can actually open, from the wraps alone.
    ///
    /// Deliberately does not consult the grant log. A grant is a public
    /// statement of intent; what a reader can read is decided by which keys they
    /// hold, and the two are only equal if the mechanism is honest.
    pub fn read_as(&self, reader: &Identity) -> Result<BTreeMap<i64, Vec<Record>>, VaultError> {
        let name = format!("{}.wrap", hex(&reader.enc_public()));
        let mut out = BTreeMap::new();
        for epoch in self.epochs()? {
            let path = self.root.join("wraps").join(epoch.to_string()).join(&name);
            if !path.exists() {
                continue;
            }
            let wrapped = fs::read(path)?;
            let Ok(key) = unwrap(&wrapped, epoch, &reader.encryption) else { continue };
            let sealed = fs::read(self.root.join("epochs").join(format!("{epoch}.seal")))?;
            let Ok(plain) = open_epoch(&sealed, &key, epoch, &self.subject_pub) else { continue };
            let text = String::from_utf8_lossy(&plain);
            let records: Vec<Record> = text
                .lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| Record::from_json(l).ok())
                .filter(|r| r.kind() != crate::kind::META)
                .collect();
            out.insert(epoch, records);
        }
        Ok(out)
    }
}

/// Group records into UTC-day epochs, dropping the header.
pub fn by_epoch(records: &[Record]) -> BTreeMap<i64, Vec<Record>> {
    let mut out: BTreeMap<i64, Vec<Record>> = BTreeMap::new();
    for r in records {
        if r.kind() == crate::kind::META {
            continue;
        }
        out.entry(epoch_of(r.t())).or_default().push(r.clone());
    }
    out
}
