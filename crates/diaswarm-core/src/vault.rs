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
//!   readers.json                   the subject's private book: tag -> reader key
//! ```
//!
//! **`readers.json` NEVER LEAVES.** It is the one file here that would undo
//! D13: the grant log names nobody precisely so that a holder learns nothing
//! about who reads, and this book is the mapping back. Nothing in
//! `diaswarm-net` serves it — a manifest lists segments, and every other
//! request names a specific file — and a replica of a vault does not contain
//! it. It exists because a grant made today has to keep wrapping tomorrow's
//! segments, and wrapping needs the reader's public key.
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

use crate::seal::{epoch_key, grant_tag, open_epoch, seal_epoch, unwrap, wrap, KEY_BYTES};
use crate::{encode, encode_records, epoch_of, Record, SPEC_VERSION};

/// A freshly created segment holds only its header.
fn existing_was_empty(plaintext: &[u8]) -> bool {
    plaintext.split(|b| *b == b'\n').filter(|l| !l.is_empty()).count() <= 1
}

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
    /// Position in the chain. Signed, so entries cannot be reordered.
    pub seq: u64,
    /// Who, and for what — as a tag only. See [`crate::seal::grant_tag`].
    ///
    /// **Not the reader's key and not the purpose.** Those made the log a
    /// published social graph: a reader's key is the same key in every
    /// subject's log, so one clinician granted by fifty people appeared
    /// identically fifty times. The tag is derived from the shared secret, so
    /// it differs per subject, the reader can still compute their own, and
    /// nobody else can compute either.
    pub tag: String,
    pub act: String,
    /// The segment this statement takes effect from, inclusive.
    ///
    /// A segment, not an epoch, so a withdrawal can land mid-day. Segments are
    /// globally monotonic, so "the latest statement at or before this segment"
    /// is a total order with no ties to break.
    pub segment: u64,
    /// Hash of the previous entry, so removing one is detectable.
    ///
    /// §11 promises the grant record is tamper-evident, and the obvious attack
    /// is not editing an entry but deleting one — a subject denying a grant
    /// they made. A signature per entry does not catch that; a chain does.
    pub prev: String,
    pub sig: String,
}

fn grant_payload(seq: u64, tag: &str, act: &str, segment: u64, prev: &str) -> Vec<u8> {
    // Sorted keys, no spaces — the same canonical shape as a record, so what is
    // signed is exactly what is on disk.
    format!(
        r#"{{"act":"{act}","prev":"{prev}","segment":{segment},"seq":{seq},"tag":"{tag}"}}"#
    )
    .into_bytes()
}

const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn chain_hash(payload: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex(&Sha256::digest(payload))
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

/// What a rewrap pass did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Rewrap {
    /// Wraps written that were missing.
    pub written: usize,
    /// Segments a reader is entitled to whose key could not be read. Any of
    /// these means the vault has lost data — that segment opens for nobody.
    pub unwrappable: usize,
}

/// Which segments a tag is entitled to: the latest statement at or before a
/// segment decides it, and only a grant makes it live.
fn live_segments(grants: &[Grant], segments: &[SegmentId], tag: &str) -> BTreeSet<u64> {
    let mut mine: Vec<&Grant> = grants.iter().filter(|g| g.tag == tag).collect();
    mine.sort_by_key(|g| g.segment);

    let mut live = BTreeSet::new();
    for seg in segments {
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
    live
}

/// One remembered reader: the tag they are filed under and the key to wrap to.
///
/// The purpose is kept only so a subject can be told who is who; the tag is
/// what everything else is keyed by, and it already fixes the purpose.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownReader {
    pub tag: String,
    pub reader: String,
    pub purpose: String,
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

        // APPEND, DO NOT REPLACE. A segment is written to repeatedly as its
        // records arrive, and a caller passes what it has just collected — not
        // the whole day. Writing that over the segment destroys everything
        // sealed into it earlier, silently: the subject loses history from the
        // only copy they have, and a reader who could read it yesterday cannot
        // today, with no error anywhere. Found by watching a reader's record
        // count fall from 32,150 to 27,874 between two fetches.
        let path = self.root.join("segments").join(seg.seal_name());
        let existing = match fs::read(&path) {
            Ok(sealed) => open_epoch(&sealed, &key, epoch, &self.subject_pub)
                .map_err(|_| VaultError::Malformed("a sealed segment did not open".into()))?,
            Err(_) => Vec::new(),
        };

        let mut plaintext = if existing.is_empty() {
            encode(&[], self.offset).into_bytes()
        } else {
            existing
        };

        // Skip anything already in the segment. The emitter dedupes within one
        // pass and the high-water marks stop a record being drained twice, but a
        // full resync would otherwise append the whole history again.
        let seen: std::collections::HashSet<&[u8]> =
            plaintext.split(|b| *b == b'\n').collect();
        let addition: Vec<Record> = records
            .iter()
            .filter(|r| !seen.contains(r.to_canonical_json().as_bytes()))
            .cloned()
            .collect();
        if addition.is_empty() && !existing_was_empty(&plaintext) {
            // Nothing new to seal, but the wraps may still be behind — this is
            // the path a repair takes when a sync finds no fresh records.
            self.rewrap()?;
            return Ok(seg);
        }
        plaintext.extend_from_slice(encode_records(&addition).as_bytes());

        let sealed = seal_epoch(&plaintext, &key, epoch, &self.subject_pub);
        fs::write(&path, sealed)?;

        // WRAP THE SEGMENT FOR EVERYONE ENTITLED TO IT, HERE, EVERY TIME.
        //
        // Wrapping used to happen only when a grant was made, so a reader
        // received exactly the segments that existed at that instant and
        // nothing after. A new segment is cut at every epoch boundary, so a
        // follower granted today silently stopped receiving data tomorrow:
        // segments kept arriving, none of them openable, and no error anywhere
        // to say so. Observed in the field as 123 segments fetched, 0 wraps.
        //
        // AFTER the write, not before: a segment is discovered by its `.seal`
        // file, so a brand-new one is invisible to `segments()` until this
        // point. Calling it earlier wrapped every segment except the one just
        // created — which is the same failure one day late, and is exactly
        // what the first version of this fix did.
        //
        // This is the only place segments come into existence (`rotate` merely
        // empties the open map; the next seal cuts the new ones), which is
        // what makes "entitled to" and "has a wrap for" the same set.
        self.rewrap()?;
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

    /// Record a grant or withdrawal. Returns the tag it was filed under, which
    /// the caller may want for a private note of who that is.
    pub fn record_grant(
        &self,
        subject: &Identity,
        reader_pub: &[u8; 32],
        purpose: &str,
        act: &str,
        segment: u64,
    ) -> Result<String, VaultError> {
        let tag = hex(&grant_tag(&subject.encryption, reader_pub, purpose));

        // The subject's private note of who this tag is, so segments cut after
        // today can still be wrapped for them. Written for a withdrawal too:
        // `entitled` is what decides whether anything gets wrapped, so keeping
        // the key costs nothing and lets a later re-grant work.
        self.remember_reader(&tag, reader_pub, purpose)?;

        let existing = self.grants()?;

        // IDEMPOTENT. Re-stating a grant that already stands appends nothing.
        // The log is hash-chained and permanent, so a settings toggle applied
        // twice — or a grant re-issued to repair missing wraps — would
        // otherwise grow it with lines that say nothing new. The wrapping below
        // still runs, which is what makes re-granting a usable repair.
        if let Some(last) = existing.iter().filter(|g| g.tag == tag).next_back() {
            if last.act == act && last.segment == segment {
                return Ok(tag);
            }
        }

        let seq = existing.len() as u64;
        let prev = match existing.last() {
            Some(last) => chain_hash(&grant_payload(
                last.seq, &last.tag, &last.act, last.segment, &last.prev,
            )),
            None => GENESIS.to_string(),
        };
        let payload = grant_payload(seq, &tag, act, segment, &prev);
        let sig: Signature = subject.signing.sign(&payload);
        let grant = Grant {
            seq,
            tag: tag.clone(),
            act: act.into(),
            segment,
            prev,
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
        Ok(tag)
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
        let payload = grant_payload(grant.seq, &grant.tag, &grant.act, grant.segment, &grant.prev);
        let Ok(bytes) = unhex(&grant.sig) else { return false };
        let Ok(sig) = Signature::from_slice(&bytes) else { return false };
        subject.verify(&payload, &sig).is_ok()
    }

    /// Check the whole log: every signature, and every link.
    ///
    /// Catches an entry that was **altered or removed from the middle**, which
    /// a per-entry signature alone does not.
    ///
    /// **It does not catch a truncated tail — locally.** What remains after
    /// dropping the last entries is a valid prefix, and the subject holds every
    /// key needed to re-sign a shorter log.
    ///
    /// **Replication is what closes that, and it is not a new mechanism.** Once
    /// the log has reached peers, truncating the local copy is no longer
    /// deletion but *equivocation*: this copy says one thing, theirs says
    /// another, and the disagreement is the evidence. feasibility.md §7.4 lists
    /// "publication is permanent" as a **cost** — you cannot delete your data.
    /// Applied to the grant log it is the **benefit**: you cannot delete your
    /// grants either. Same property, read from the other side.
    ///
    /// **Which makes it a requirement, not a hope: the grant log must replicate
    /// to peers, not only the sealed data.** It is a few hundred bytes an entry,
    /// so the cost is nothing, and without it the tamper-evidence §11 promises
    /// rests on the subject's own copy being honest.
    ///
    /// Two limits survive replication and should not be talked past. An entry
    /// created and dropped **before any peer saw it** leaves no trace anywhere —
    /// the guarantee is "what was seen is permanent", never "the log is
    /// complete". And a subject can show **different logs to different peers**;
    /// catching that needs peers to compare with each other, which is what
    /// Certificate Transparency calls gossip and what this design gets for free
    /// only if peers actually do it.
    ///
    /// Returns the position of the first break.
    pub fn verify_chain(&self, subject: &VerifyingKey) -> Result<Option<u64>, VaultError> {
        let mut prev = GENESIS.to_string();
        for (i, g) in self.grants()?.iter().enumerate() {
            if g.seq != i as u64 || g.prev != prev || !Self::verify(g, subject) {
                return Ok(Some(i as u64));
            }
            prev = chain_hash(&grant_payload(g.seq, &g.tag, &g.act, g.segment, &g.prev));
        }
        Ok(None)
    }

    /// Which segments a reader is currently entitled to.
    ///
    /// A segment is live if the latest statement at or before it was a grant.
    /// Entitlement is a property of the signed log, not of anything a server
    /// decides.
    pub fn entitled(&self, tag: &str) -> Result<BTreeSet<u64>, VaultError> {
        Ok(live_segments(&self.grants()?, &self.segments()?, tag))
    }

    /// The subject's private book of who reads. Empty if there is none.
    pub fn readers(&self) -> Result<Vec<KnownReader>, VaultError> {
        let path = self.root.join("readers.json");
        if !path.exists() {
            return Ok(Vec::new());
        }
        serde_json::from_slice(&fs::read(path)?).map_err(|e| VaultError::Malformed(e.to_string()))
    }

    /// Note a reader's key against their tag, so later segments can be wrapped.
    ///
    /// An upsert keyed by tag, and the tag already fixes the purpose, so
    /// re-granting the same reader the same thing rewrites one entry rather
    /// than accumulating them.
    pub fn remember_reader(
        &self,
        tag: &str,
        reader_pub: &[u8; 32],
        purpose: &str,
    ) -> Result<(), VaultError> {
        let mut book = self.readers()?;
        let entry = KnownReader {
            tag: tag.to_string(),
            reader: hex(reader_pub),
            purpose: purpose.to_string(),
        };
        match book.iter_mut().find(|k| k.tag == tag) {
            Some(slot) => *slot = entry,
            None => book.push(entry),
        }
        let path = self.root.join("readers.json");
        fs::write(&path, serde_json::to_vec_pretty(&book).unwrap())?;
        // The book is the one file here that undoes D13's unlinkability, so it
        // is not left readable the way a sealed segment safely is.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    /// Bring every remembered reader's wraps up to date with the segments.
    ///
    /// Needs no identity: a wrap is made with an ephemeral key against the
    /// reader's public one, so the subject's secret is not involved. That is
    /// what lets [`Vault::seal`] call this on every write.
    pub fn rewrap(&self) -> Result<Rewrap, VaultError> {
        let book = self.readers()?;
        if book.is_empty() {
            return Ok(Rewrap::default());
        }
        let segments = self.segments()?;
        let grants = self.grants()?;
        let mut total = Rewrap::default();
        for known in &book {
            let Ok(bytes) = unhex(&known.reader) else { continue };
            let Ok(reader_pub) = <[u8; 32]>::try_from(bytes.as_slice()) else { continue };
            let one = self.wrap_for(&known.tag, &reader_pub, &grants, &segments)?;
            total.written += one.written;
            total.unwrappable += one.unwrappable;
        }
        Ok(total)
    }

    /// Wrap every segment one tag is entitled to and has not been given.
    fn wrap_for(
        &self,
        tag: &str,
        reader_pub: &[u8; 32],
        grants: &[Grant],
        segments: &[SegmentId],
    ) -> Result<Rewrap, VaultError> {
        let live = live_segments(grants, segments, tag);
        let mut out = Rewrap::default();
        for seg in segments {
            if !live.contains(&seg.seq) {
                continue;
            }
            let dir = self.root.join("wraps").join(seg.seq.to_string());
            let path = dir.join(format!("{tag}.wrap"));
            if path.exists() {
                continue; // a wrap is not re-issued
            }
            // A SEGMENT WHOSE KEY IS GONE IS SKIPPED, NOT FATAL. Without its
            // key that segment is unreadable by everyone, the subject included
            // — the loss already happened, and refusing to seal from now on
            // would turn one lost day into every future one. Counted, so a
            // caller can say so rather than quietly wrapping less than it
            // claimed.
            let Ok(key) = self.segment_key(seg) else {
                out.unwrappable += 1;
                continue;
            };
            fs::create_dir_all(&dir)?;
            fs::write(path, wrap(&key, seg.epoch, reader_pub))?;
            out.written += 1;
        }
        Ok(out)
    }

    /// Wrap every segment this reader is entitled to and has not been sent,
    /// remembering them so that later segments are wrapped too.
    pub fn publish_wraps(
        &self,
        subject: &Identity,
        reader_pub: &[u8; 32],
        purpose: &str,
    ) -> Result<usize, VaultError> {
        // The wrap FILENAME used to be the reader's public key, which leaked
        // exactly what the grant log stopped leaking. Same tag, same reasoning.
        let tag = hex(&grant_tag(&subject.encryption, reader_pub, purpose));
        self.remember_reader(&tag, reader_pub, purpose)?;
        let out = self.wrap_for(&tag, reader_pub, &self.grants()?, &self.segments()?)?;
        Ok(out.written)
    }

    /// Everything a reader can actually open, grouped by epoch.
    ///
    /// Deliberately does not consult the grant log. A grant is a public
    /// statement of intent; what a reader can read is decided by which keys they
    /// hold, and the two are only equal if the mechanism is honest.
    /// What this reader can open, for one purpose.
    ///
    /// The reader derives the same tag from their own side of the shared
    /// secret, so they find their wraps without the log naming them.
    pub fn read_as(
        &self,
        reader: &Identity,
        purpose: &str,
    ) -> Result<BTreeMap<i64, Vec<Record>>, VaultError> {
        let tag = hex(&grant_tag(&reader.encryption, &self.subject_pub, purpose));
        let name = format!("{tag}.wrap");
        let mut out: BTreeMap<i64, Vec<Record>> = BTreeMap::new();

        // SEGMENTS CAN LEGITIMATELY OVERLAP, so a reader assembling a history
        // has to deduplicate. A rotation starts a new segment for the same
        // epoch, and a resync writes records into it that an earlier segment
        // already held — neither is a fault, and a reader that trusted the
        // segments to be disjoint would double-count insulin. Which is exactly
        // what §3 exists to prevent, arriving from a direction the snapshot
        // filters cannot see. Observed as 61,339 records where 33,000 were
        // expected.
        let mut seen: BTreeSet<String> = BTreeSet::new();
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
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                let Ok(record) = Record::from_json(line) else { continue };
                if record.kind() == crate::kind::META {
                    continue;
                }
                if !seen.insert(record.to_canonical_json()) {
                    continue;
                }
                out.entry(seg.epoch).or_default().push(record);
            }
        }
        for records in out.values_mut() {
            crate::sort(records);
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

/// Many vaults, one per subject, in one directory.
///
/// **This is what makes a swarm out of a set of peers.** A node that serves
/// only its own vault is a personal server: reachable when its owner's phone is
/// awake and not otherwise. A node that also serves the vaults it has
/// replicated is a peer, and a reader can get a subject's history from anyone
/// who holds it.
///
/// feasibility.md §9.2: who *holds* the bytes should be as many peers as
/// possible, because holders learn nothing — they hold ciphertext. Who can
/// *read* is decided entirely by key distribution, and that does not change
/// when the bytes move.
///
/// ```text
/// store/
///   552f688a…/    a subject's vault: meta.json, segments/, wraps/, grants
///   29daa5bf…/    another's, replicated because someone here follows them
/// ```
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, VaultError> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        Ok(Store { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where one subject's vault lives.
    ///
    /// The subject's public key is the name, so two peers independently
    /// replicating the same subject agree on the path without coordinating.
    pub fn path_for(&self, subject_pub: &[u8; 32]) -> PathBuf {
        self.root.join(hex(subject_pub))
    }

    /// Every subject this node holds anything for.
    pub fn subjects(&self) -> Result<Vec<[u8; 32]>, VaultError> {
        let mut out = Vec::new();
        if !self.root.exists() {
            return Ok(out);
        }
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if !entry.path().join("meta.json").exists() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if let Ok(bytes) = unhex(&name) {
                if let Ok(k) = <[u8; 32]>::try_from(bytes.as_slice()) {
                    out.push(k);
                }
            }
        }
        out.sort();
        Ok(out)
    }

    pub fn vault(&self, subject_pub: &[u8; 32]) -> Result<Vault, VaultError> {
        Vault::open(&self.path_for(subject_pub))
    }

    /// Adopt a vault that already exists elsewhere, once.
    ///
    /// For a node whose own vault predates the store. Moves rather than copies,
    /// because two vaults for one subject diverging is worse than either.
    pub fn adopt(&self, existing: &Path) -> Result<Option<[u8; 32]>, VaultError> {
        if !existing.join("meta.json").exists() {
            return Ok(None);
        }
        let subject = Vault::open(existing)?.subject_pub();
        let dest = self.path_for(&subject);
        if dest.exists() {
            return Ok(Some(subject));
        }
        fs::rename(existing, &dest)?;
        Ok(Some(subject))
    }
}
