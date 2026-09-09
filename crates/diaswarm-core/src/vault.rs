//! A vault: one subject's sealed history, the wraps that open it, and the
//! signed record of who was granted what.
//!
//! Files, not a swarm. Where these bytes go, how they replicate and who holds
//! them is a later stage, and nothing here has an opinion — which is the point.
//! A holder can copy a whole vault and learn nothing but its shape.
//!
//! ```text
//! vault/
//!   meta.json                      spec, epoch offset, subject public key
//!   open.json                      which segment each epoch is currently writing to
//!   segments/<seq>.<epoch>.seal    a run of records, sealed under its own key
//!   segments/<seq>.<epoch>.key     that key — the subject's copy, never shared
//!   wraps/<seq>/<reader>.wrap      that key, wrapped to one reader
//!   grants.ndjson                  signed: granted what, from which segment
//! ```
//!
//! **A SEGMENT, NOT A DAY, IS THE UNIT OF KEY CUSTODY.** An epoch is still how
//! access is scoped and addressed — grants think in days — but a day can hold
//! several segments, and [`Vault::rotate`] cuts a new one on demand.
//!
//! That exists because revocation used to wait for the epoch boundary. At
//! UTC+12 that meant revoking at 9pm left a reader everything until noon the
//! next day: fifteen hours of future data after the subject said stop, and the
//! worst case landing in the middle of the waking day. Rotating on revocation
//! makes it immediate, and leaves epoch length a question of cost and scoping
//! rather than of safety.
//!
//! **Revocation is still the absence of a file.** No delete step and no message
//! to anyone: the reader is simply not wrapped for the new segment. What they
//! already fetched keeps working, because nothing here can reach into a copy
//! someone already has — and claiming otherwise is the recall feasibility.md
//! §11 forbids implying.

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
    /// The fixed per-subject offset epochs are cut at. Recorded once, because
    /// re-cutting a history at a different phase silently reshapes every daily
    /// figure computed from it.
    offset: i64,
    subject: String,
}

/// A segment: a run of records sealed under one key.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct SegmentId {
    pub seq: u64,
    pub epoch: i64,
}

impl SegmentId {
    fn seal_name(&self) -> String {
        format!("{}.{}.seal", self.seq, self.epoch)
    }
    fn key_name(&self) -> String {
        format!("{}.{}.key", self.seq, self.epoch)
    }
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
    /// The segment this statement takes effect from, inclusive.
    ///
    /// A segment, not an epoch, so a withdrawal can land mid-day. Segments are
    /// globally monotonic, so "the latest statement at or before this segment"
    /// is a total order with no ties to break.
    pub segment: u64,
    pub purpose: String,
    pub reader: String,
    pub subject: String,
    pub sig: String,
}

fn grant_payload(act: &str, segment: u64, purpose: &str, reader: &str, subject: &str) -> Vec<u8> {
    // Sorted keys, no spaces — the same canonical shape as a record, so what is
    // signed is exactly what is on disk.
    format!(
        r#"{{"act":"{act}","purpose":"{purpose}","reader":"{reader}","segment":{segment},"subject":"{subject}"}}"#
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
    offset: i64,
}

impl Vault {
    pub fn create(root: &Path, subject: &Identity, offset: i64) -> Result<Self, VaultError> {
        fs::create_dir_all(root.join("segments"))?;
        fs::create_dir_all(root.join("wraps"))?;
        let meta = Meta {
            spec: SPEC_VERSION,
            epoch: crate::EPOCH_BASIS.to_string(),
            offset,
            subject: hex(&subject.enc_public()),
        };
        fs::write(root.join("meta.json"), serde_json::to_vec_pretty(&meta).unwrap())?;
        Ok(Vault { root: root.to_path_buf(), subject_pub: subject.enc_public(), offset })
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
        Ok(Vault { root: root.to_path_buf(), subject_pub, offset: meta.offset })
    }

    pub fn subject_pub(&self) -> [u8; 32] {
        self.subject_pub
    }

    /// The fixed offset this vault's epochs are cut at.
    pub fn offset(&self) -> i64 {
        self.offset
    }

    /// Every segment, in order.
    pub fn segments(&self) -> Result<Vec<SegmentId>, VaultError> {
        let mut out = Vec::new();
        for entry in fs::read_dir(self.root.join("segments"))? {
            let name = entry?.file_name().to_string_lossy().to_string();
            let Some(stem) = name.strip_suffix(".seal") else { continue };
            let mut parts = stem.splitn(2, '.');
            let (Some(seq), Some(epoch)) = (parts.next(), parts.next()) else { continue };
            if let (Ok(seq), Ok(epoch)) = (seq.parse::<u64>(), epoch.parse::<i64>()) {
                out.push(SegmentId { seq, epoch });
            }
        }
        out.sort();
        Ok(out)
    }

    /// The epochs this vault holds anything for.
    pub fn epochs(&self) -> Result<Vec<i64>, VaultError> {
        let mut out: Vec<i64> = self.segments()?.into_iter().map(|s| s.epoch).collect();
        out.sort_unstable();
        out.dedup();
        Ok(out)
    }

    /// The seq a new segment would take.
    pub fn next_seq(&self) -> Result<u64, VaultError> {
        Ok(self.segments()?.last().map(|s| s.seq + 1).unwrap_or(0))
    }

    fn open_map(&self) -> Result<BTreeMap<i64, u64>, VaultError> {
        let path = self.root.join("open.json");
        if !path.exists() {
            return Ok(BTreeMap::new());
        }
        serde_json::from_slice(&fs::read(path)?).map_err(|e| VaultError::Malformed(e.to_string()))
    }

    fn write_open_map(&self, m: &BTreeMap<i64, u64>) -> Result<(), VaultError> {
        fs::write(self.root.join("open.json"), serde_json::to_vec(m).unwrap())?;
        Ok(())
    }

    /// Close every open segment. The next seal starts fresh ones.
    ///
    /// THIS IS WHAT MAKES REVOCATION IMMEDIATE. Everything sealed so far stays
    /// in segments a departing reader is already wrapped for; everything after
    /// lands in a segment they are not.
    pub fn rotate(&self) -> Result<(), VaultError> {
        self.write_open_map(&BTreeMap::new())
    }

    /// Seal records into the open segment for their epoch, opening one if needed.
    ///
    /// Re-sealing a segment reuses its key, because a segment is written to
    /// repeatedly as its records arrive and minting a fresh key would silently
    /// invalidate every wrap already published for it — readers left holding a
    /// key that opens nothing, with no error anywhere to say so.
    pub fn seal(&self, epoch: i64, records: &[Record]) -> Result<SegmentId, VaultError> {
        let mut open = self.open_map()?;
        let seg = match open.get(&epoch) {
            Some(seq) => SegmentId { seq: *seq, epoch },
            None => {
                let seq = self.next_seq()?;
                open.insert(epoch, seq);
                self.write_open_map(&open)?;
                SegmentId { seq, epoch }
            }
        };

        let key = match self.segment_key(&seg) {
            Ok(existing) => existing,
            Err(_) => {
                let k = epoch_key();
                fs::write(self.root.join("segments").join(seg.key_name()), k)?;
                k
            }
        };

        let plaintext = encode(records, self.offset);
        let sealed = seal_epoch(plaintext.as_bytes(), &key, epoch, &self.subject_pub);
        fs::write(self.root.join("segments").join(seg.seal_name()), sealed)?;
        Ok(seg)
    }

    fn segment_key(&self, seg: &SegmentId) -> Result<[u8; KEY_BYTES], VaultError> {
        let raw = fs::read(self.root.join("segments").join(seg.key_name()))?;
        if raw.len() != KEY_BYTES {
            return Err(VaultError::Malformed("segment key is the wrong length".into()));
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
        segment: u64,
    ) -> Result<(), VaultError> {
        let reader = hex(reader_pub);
        let subj = hex(&self.subject_pub);
        let payload = grant_payload(act, segment, purpose, &reader, &subj);
        let sig: Signature = subject.signing.sign(&payload);
        let grant = Grant {
            act: act.into(),
            segment,
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

    /// Withdraw, immediately.
    ///
    /// Rotates first so that everything written from here lands in a segment
    /// this reader is not wrapped for, then records the withdrawal from that
    /// segment. Without the rotation the reader would keep receiving the rest
    /// of the current segment — which, at UTC+12, could be most of a day.
    pub fn revoke(
        &self,
        subject: &Identity,
        reader_pub: &[u8; 32],
        purpose: &str,
    ) -> Result<u64, VaultError> {
        self.rotate()?;
        let from = self.next_seq()?;
        self.record_grant(subject, reader_pub, purpose, "stop", from)?;
        Ok(from)
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

    pub fn verify(grant: &Grant, subject: &VerifyingKey) -> bool {
        let payload = grant_payload(
            &grant.act,
            grant.segment,
            &grant.purpose,
            &grant.reader,
            &grant.subject,
        );
        let Ok(bytes) = unhex(&grant.sig) else { return false };
        let Ok(sig) = Signature::from_slice(&bytes) else { return false };
        subject.verify(&payload, &sig).is_ok()
    }

    /// Which segments a reader is currently entitled to.
    ///
    /// A segment is live if the latest statement at or before it was a grant.
    /// Entitlement is a property of the signed log, not of anything a server
    /// decides.
    pub fn entitled(
        &self,
        reader_pub: &[u8; 32],
        purpose: &str,
    ) -> Result<BTreeSet<u64>, VaultError> {
        let reader = hex(reader_pub);
        let mut mine: Vec<Grant> = self
            .grants()?
            .into_iter()
            .filter(|g| g.reader == reader && g.purpose == purpose)
            .collect();
        mine.sort_by_key(|g| g.segment);

        let mut live = BTreeSet::new();
        for seg in self.segments()? {
            let mut state: Option<&str> = None;
            for g in &mine {
                if g.segment <= seg.seq {
                    state = Some(&g.act);
                }
            }
            if state == Some("grant") {
                live.insert(seg.seq);
            }
        }
        Ok(live)
    }

    /// Wrap every segment this reader is entitled to and has not been sent.
    pub fn publish_wraps(&self, reader_pub: &[u8; 32], purpose: &str) -> Result<usize, VaultError> {
        let reader = hex(reader_pub);
        let entitled = self.entitled(reader_pub, purpose)?;
        let mut written = 0;
        for seg in self.segments()? {
            if !entitled.contains(&seg.seq) {
                continue;
            }
            let dir = self.root.join("wraps").join(seg.seq.to_string());
            fs::create_dir_all(&dir)?;
            let path = dir.join(format!("{reader}.wrap"));
            if path.exists() {
                continue; // a wrap is not re-issued
            }
            let key = self.segment_key(&seg)?;
            fs::write(path, wrap(&key, seg.epoch, reader_pub))?;
            written += 1;
        }
        Ok(written)
    }

    /// Everything a reader can actually open, grouped by epoch.
    ///
    /// Deliberately does not consult the grant log. A grant is a public
    /// statement of intent; what a reader can read is decided by which keys they
    /// hold, and the two are only equal if the mechanism is honest.
    pub fn read_as(&self, reader: &Identity) -> Result<BTreeMap<i64, Vec<Record>>, VaultError> {
        let name = format!("{}.wrap", hex(&reader.enc_public()));
        let mut out: BTreeMap<i64, Vec<Record>> = BTreeMap::new();
        for seg in self.segments()? {
            let path = self.root.join("wraps").join(seg.seq.to_string()).join(&name);
            if !path.exists() {
                continue;
            }
            let wrapped = fs::read(path)?;
            let Ok(key) = unwrap(&wrapped, seg.epoch, &reader.encryption) else { continue };
            let sealed = fs::read(self.root.join("segments").join(seg.seal_name()))?;
            let Ok(plain) = open_epoch(&sealed, &key, seg.epoch, &self.subject_pub) else { continue };
            let text = String::from_utf8_lossy(&plain);
            let records = text
                .lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| Record::from_json(l).ok())
                .filter(|r| r.kind() != crate::kind::META);
            out.entry(seg.epoch).or_default().extend(records);
        }
        Ok(out)
    }
}

/// Group records into epochs at a given offset, dropping the header.
pub fn by_epoch(records: &[Record], offset: i64) -> BTreeMap<i64, Vec<Record>> {
    let mut out: BTreeMap<i64, Vec<Record>> = BTreeMap::new();
    for r in records {
        if r.kind() == crate::kind::META {
            continue;
        }
        out.entry(epoch_of(r.t(), offset)).or_default().push(r.clone());
    }
    out
}
