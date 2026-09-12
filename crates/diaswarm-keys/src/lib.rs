//! The vault of [D26](../../docs/decisions.md): p2panda's keys, diaswarm's segments.
//!
//! **WHAT THIS IS FOR.** `diaswarm-core` reads the recent end of a history in
//! constant time and is 1,159 lines of cryptography nobody has reviewed.
//! `diaswarm-spaces` deletes that code and cannot express a follower who reads
//! only the last day, because `SpacesArgs::Application` chains to its space's
//! previous tips — measured at 32 ms a day and rising after six weeks.
//!
//! This takes the key layer from `p2panda-encryption` and keeps the segment
//! layout. Membership and secret rotation are the library's: `create`, `add`,
//! `remove`, and a welcome that hands a joiner the whole secret bundle. A
//! segment is `encrypt_data` under the current group secret, written to disk
//! beside the id of the secret that opens it — so reading epoch N needs segment
//! N and nothing else, and a reader who wants a day reads a day.
//!
//! **WHAT IS DELIBERATELY NOT HERE.** Records never become group messages.
//! `EncryptionGroup::send` is not called and the application message variant in
//! [`group::Message`] is never constructed, which is what keeps the control
//! history one entry per grant instead of one per reading.
//!
//! Auth — who may grant, and the tamper-evident record of grants ([D13]) — is
//! [`auth`]. It turned out to be mostly already here: a p2panda log is signed
//! per entry, hash-chained and replicable by construction, and the control log
//! names members by [`group::GrantTag`], so what was missing was the checking
//! rather than the structure. The log records what the subject did; it does not
//! decide what a reader may open, because here that is decided by whether the
//! reader holds the secret a segment names.
//!
//! **CONTROL MESSAGES ARE SIGNED, AND THE TYPE SYSTEM SAYS SO.**
//! [`group::Message`] is a plain struct whose `sender` field anybody can set, so
//! [`Vault::receive`] and [`Vault::join`] do not take one: they take an
//! [`Authentic`], and the only way to make an `Authentic` is
//! [`wire::open_control`], which will not return one unless a
//! `p2panda_core::Operation` carrying the message was signed by the key the
//! caller named and the message's `sender` is that key's [`group::GrantTag`].
//!
//! Three claims are checked, and the third is the one that matters:
//!
//! 1. the signature verifies and the body is the one the header committed to —
//!    `p2panda_core::validate_operation`;
//! 2. the author is the subject the caller expected, not merely *some* valid
//!    author;
//! 3. the `sender` inside the message equals `GrantTag::own(author)`, so a
//!    signed message cannot claim to come from a different member than the one
//!    that signed it.
//!
//! This is the same construction `p2panda-spaces` uses — every message is a
//! signed operation — reached by the same route, and [`wire`] now carries
//! control messages as well as segments. They go in **their own log**: mixing
//! them into the segment log would break the invariant `wire::segments_tail`
//! rests on, that the last N entries are the last N days.
//!
//! **ONLY THE SUBJECT GRANTS**, which is what makes the check that simple.
//! `GrantTag::own` is a public function of a public key, so anyone can compute
//! the one identifier a control message is allowed to carry as its sender;
//! every *other* tag in the message — the reader being added — stays
//! unlinkable, because those come from ECDH and this one deliberately does
//! not.

pub mod auth;
pub mod group;
pub mod wire;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use p2panda_core::SigningKey;
/// Re-exported so a caller needs a key bundle without also depending on
/// `p2panda-encryption` — the JNI layer wants exactly this and nothing else
/// from it.
pub use p2panda_encryption::Rng;

/// The store a control log lives in, re-exported for the same reason.
///
/// [`wire::publish_control`] takes one, so anything that grants needs the type.
/// Re-exporting keeps the exact-version pin (`=0.7.1`) in one crate instead of
/// every caller, and a mismatch there is the kind that shows up as a trait not
/// being implemented for a type that visibly implements it.
pub use p2panda_store::{SqliteStore, SqliteStoreBuilder};
use p2panda_encryption::crypto::x25519::SecretKey;
use p2panda_encryption::crypto::xchacha20::XAeadNonce;
use p2panda_encryption::data_scheme::{
    EncryptionGroup, GroupState, GroupSecretId, decrypt_data, encrypt_data,
};
use p2panda_encryption::key_bundle::{Lifetime, LongTermKeyBundle};
use p2panda_encryption::key_manager::{KeyManager, KeyManagerState};
use p2panda_encryption::key_registry::KeyRegistry;
use p2panda_encryption::traits::PreKeyManager;
use p2panda_encryption::traits::IdentityManager as _;
use serde::{Deserialize, Serialize};

use diaswarm_core::{Record, encode_records};

use p2panda_encryption::traits::GroupMembership as _;

use group::{Dgm, GrantTag, MemberId, Message, Order, OperationId};

type Group = EncryptionGroup<MemberId, OperationId, KeyRegistry<MemberId>, Dgm, KeyManager, Order>;
type State = GroupState<MemberId, OperationId, KeyRegistry<MemberId>, Dgm, KeyManager, Order>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("group: {0}")]
    Group(String),
    #[error("crypto: {0}")]
    Crypto(String),
    #[error("no group secret yet — open a group first")]
    NoSecret,
    /// A message did not prove it came from who it says it came from.
    #[error("unauthenticated control message: {0}")]
    Forged(String),
    #[error("encoding: {0}")]
    Encode(String),
    /// Sealed under a secret this vault was never given — access control
    /// working, not a failure. Counted as `not_ours`.
    #[error("sealed under a secret this vault was not given")]
    NotGranted,
    /// The secret was held and the ciphertext still would not open. A failure.
    #[error("held the secret and the segment still would not open")]
    Undecryptable,
    /// The control message was genuine and was not this vault's welcome.
    #[error("that control message does not welcome this vault into the group")]
    NotWelcomed,
    #[error(transparent)]
    Store(#[from] p2panda_store::SqliteError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// What a read could not open, so that a short answer is never a silent one.
///
/// `not_ours` is the architecture working and the others are not, which is the
/// whole reason they are counted separately.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Skipped {
    /// Sealed under a secret this vault was never given. Expected.
    pub not_ours: usize,
    /// The file would not read or would not parse as a segment.
    pub unreadable: usize,
    /// The secret was held and the ciphertext still would not open.
    pub undecryptable: usize,
    /// A line inside a decrypted segment was not a record.
    pub unparseable: usize,
}

impl Skipped {
    /// Anything here that is not access control doing its job.
    pub fn lost(&self) -> usize {
        self.unreadable + self.undecryptable + self.unparseable
    }
}

/// A sealed day, exactly as it sits on disk.
///
/// **IT NAMES ITS KEY AND NOTHING ELSE.** No backlink, no previous tips, no
/// sequence number. That is the whole difference from a spaces application
/// message, and the reason a reader can open this one without having opened any
/// other.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub epoch: i64,
    pub secret_id: GroupSecretId,
    pub nonce: XAeadNonce,
    pub ciphertext: Vec<u8>,
}

/// A key bundle as text, for an invite or a QR code.
///
/// **HEX OF CBOR, AND BOTH HALVES ARE DELIBERATE.** CBOR because that is what
/// everything else here encodes with and because JSON cannot hold the maps
/// involved; hex because an invite is a colon-separated string that gets
/// scanned off a screen, and base64 has characters that fight with both.
///
/// A bundle is a few hundred bytes, so a QR code is unbothered.
pub fn encode_bundle(bundle: &LongTermKeyBundle) -> Result<String, Error> {
    let bytes =
        p2panda_core::cbor::encode_cbor(bundle).map_err(|e| Error::Encode(e.to_string()))?;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in &bytes {
        out.push_str(&format!("{b:02x}"));
    }
    Ok(out)
}

/// The other direction. Rejects anything that is not a bundle rather than
/// producing one that will fail mysteriously at key agreement.
pub fn decode_bundle(text: &str) -> Result<LongTermKeyBundle, Error> {
    if text.len() % 2 != 0 || text.is_empty() {
        return Err(Error::Encode("a bundle is an even number of hex digits".into()));
    }
    let mut bytes = Vec::with_capacity(text.len() / 2);
    for i in (0..text.len()).step_by(2) {
        let byte = u8::from_str_radix(&text[i..i + 2], 16)
            .map_err(|_| Error::Encode("a bundle is hex".into()))?;
        bytes.push(byte);
    }
    p2panda_core::cbor::decode_cbor(&bytes[..])
        .map_err(|e| Error::Encode(format!("not a key bundle: {e}")))
}

/// A control message whose sender has been proved against a signature.
///
/// **THE POINT IS THAT THERE IS NO OTHER CONSTRUCTOR.** The field is private
/// and [`wire::open_control`] is the only thing in the crate that fills it, so
/// "did anybody check who sent this?" is answered by the type rather than by
/// reading [`Vault::receive`] carefully. A bare [`group::Message`] cannot be
/// received at all.
///
/// It carries the author it was verified against, not just the message, so
/// [`Vault::receive`] can also refuse a message that is perfectly valid but
/// from the wrong subject.
#[derive(Debug, Clone)]
pub struct Authentic {
    author: p2panda_core::VerifyingKey,
    message: Message,
}

impl Authentic {
    /// The key whose signature was checked.
    pub fn author(&self) -> &p2panda_core::VerifyingKey {
        &self.author
    }

    /// The message, now that it has been vouched for.
    pub fn message(&self) -> &Message {
        &self.message
    }

    /// Only [`wire::open_control`] calls this, and only after checking.
    pub(crate) fn vouched(author: p2panda_core::VerifyingKey, message: Message) -> Self {
        Authentic { author, message }
    }
}

pub struct Vault {
    root: PathBuf,
    offset: i64,
    state: Option<State>,
    rng: Rng,
    me: MemberId,
    /// Whose control messages this vault will accept: itself if it created the
    /// group, the subject if it joined one. `None` until either happens.
    subject_key: Option<p2panda_core::VerifyingKey>,
    /// This vault's own signing identity, so a subject vault can vouch for the
    /// messages it produced itself without a round trip through the store.
    my_key: p2panda_core::VerifyingKey,
}

impl Vault {
    /// Open or create a vault at `root`, restoring its group state if it has one.
    ///
    /// **A VAULT THAT COMES BACK AS A NEW MEMBER INVALIDATES EVERY GRANT EVER
    /// MADE TO IT**, which is why this is not optional and why the state file is
    /// 0600. [D20](../../docs/decisions.md) records the same hazard for
    /// `diaswarm-spaces`: `SecretKey::from_bytes` is `test_utils` only, so an
    /// encryption identity cannot be rebuilt from a signing key — the manager
    /// state itself has to survive, secrets and all.
    pub fn open(root: impl AsRef<Path>, offset: i64, signing: &SigningKey) -> Result<Self, Error> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(root.join("segments"))?;
        let rng = Rng::default();
        let me = GrantTag::own(&signing.verifying_key());
        let my_key = signing.verifying_key();
        let (state, subject_key) = match Self::load(&root)? {
            Some((state, key)) => (Some(state), key),
            None => (None, None),
        };
        Ok(Vault { root, offset, state, rng, me, subject_key, my_key })
    }

    /// Where the group state lives. One file, beside the segments it opens.
    fn state_path(root: &Path) -> PathBuf {
        root.join("group.cbor")
    }

    #[allow(clippy::type_complexity)]
    fn load(root: &Path) -> Result<Option<(State, Option<p2panda_core::VerifyingKey>)>, Error> {
        let path = Self::state_path(root);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path)?;
        let saved: Persisted = p2panda_core::cbor::decode_cbor(&bytes[..])
            .map_err(|e| Error::Crypto(format!("state will not decode: {e}")))?;
        let subject_key = saved.subject_key;
        Ok(Some((saved.into_state(), subject_key)))
    }

    /// Write the group state out, readable by nobody else.
    ///
    /// **IT HOLDS SECRET KEY MATERIAL** — the key manager's identity secret and
    /// every group secret this vault has been told about — so it is written
    /// 0600 and never anywhere but the vault's own directory. Losing it is
    /// losing the ability to read anything; leaking it is handing over every
    /// segment the vault can open.
    fn save(&self) -> Result<(), Error> {
        let Some(state) = self.state.as_ref() else { return Ok(()) };
        let path = Self::state_path(&self.root);
        let tmp = path.with_extension("cbor.tmp");
        // **CBOR, NOT JSON, AND NOT BY PREFERENCE.** The state holds maps keyed
        // by `VerifyingKey` — the 2SM handlers, the key registry — and a JSON
        // object key must be a string, so `serde_json` refuses them with "key
        // must be a string". CBOR has no such restriction and is what p2panda
        // encodes with everywhere else.
        let encoded = p2panda_core::cbor::encode_cbor(&PersistedRef::of(state, self.subject_key.as_ref()))
            .map_err(|e| Error::Crypto(format!("state will not encode: {e}")))?;
        std::fs::write(&tmp, encoded)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        // Renamed rather than written in place: a vault interrupted mid-write
        // would otherwise come back with a truncated state, which is the same
        // as coming back as a stranger.
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// This vault's own member id inside its own group.
    ///
    /// A subject is a member of its own group and needs a tag like anybody
    /// else. Its counterpart is itself, so the shared secret is its identity
    /// key agreed with its own public half.
    pub fn subject(&self) -> MemberId {
        self.me
    }

    /// The tag naming the relationship between this vault and `their_bundle`.
    ///
    /// Both sides compute it from their own secret and the other's published
    /// identity key, so neither has to be told, and it differs for every pair.
    pub fn tag_for(
        my_keys: &p2panda_encryption::key_manager::KeyManagerState,
        their_bundle: &LongTermKeyBundle,
        purpose: &str,
    ) -> Result<GrantTag, Error> {
        use p2panda_encryption::traits::KeyBundle as _;
        let mine = KeyManager::identity_secret(my_keys);
        let shared = mine
            .calculate_agreement(their_bundle.identity_key())
            .map_err(|e| Error::Crypto(e.to_string()))?;
        Ok(GrantTag(diaswarm_core::seal::grant_tag_from_shared(&shared, purpose)))
    }

    /// Where this vault lives. A caller that holds one vault and needs to open
    /// another beside it — a follower joining a second subject — would
    /// otherwise have to be told the path twice and could be told two
    /// different ones.
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn offset(&self) -> i64 {
        self.offset
    }

    /// This vault's long-term key bundle, for somebody else to register.
    ///
    /// The same exchange gate 6 found `p2panda-spaces` needs and the invite does
    /// not carry: a peer publishes a bundle, and whoever wants to encrypt
    /// towards them registers it.
    /// **THE IDENTITY CANNOT BE DERIVED FROM THE SIGNING KEY**, and that is the
    /// same wall [D20](../../docs/decisions.md) hit with spaces:
    /// `SecretKey::from_bytes` is `test_utils` only, so an encryption identity
    /// can be neither exported nor restored as bytes. It is generated from the
    /// Rng and the manager state is serialised instead — which is exactly what
    /// `diaswarm-spaces` does with its 0600 `credentials.json`, and for the same
    /// reason: a vault that came back from a reboot as a new member would
    /// invalidate every grant ever made to it.
    ///
    /// `init_and_generate_prekey` is also `test_utils` only; the production
    /// route is `init` then `rotate_prekey`.
    pub fn key_bundle(rng: &Rng) -> Result<(KeyManagerState, LongTermKeyBundle), Error> {
        let secret = SecretKey::from_rng(rng).map_err(|e| Error::Crypto(e.to_string()))?;
        let manager = KeyManager::init(&secret).map_err(|e| Error::Crypto(e.to_string()))?;
        let manager = KeyManager::rotate_prekey(manager, Lifetime::default(), rng)
            .map_err(|e| Error::Crypto(e.to_string()))?;
        let bundle = KeyManager::prekey_bundle(&manager).map_err(|e| Error::Crypto(e.to_string()))?;
        Ok((manager, bundle))
    }

    /// **THIS VAULT'S OWN BUNDLE, FOR PUTTING IN AN INVITE.**
    ///
    /// [`Vault::key_bundle`] generates a *new* identity every call — it is for
    /// creating a vault, not for describing one. Once a vault exists its
    /// identity lives in the persisted key manager, and this is how to ask for
    /// the public half of it.
    ///
    /// **A GRANT CANNOT HAPPEN WITHOUT BOTH SIDES' BUNDLES**, which is what
    /// makes this the blocker for the whole cutover. The subject calls
    /// [`Vault::grant`] with the *reader's* bundle; the reader needs the
    /// *subject's* to derive the same [`GrantTag`] and to open its welcome.
    /// `diaswarm-core`'s invite carries an X25519 key that serves the same
    /// purpose for the old vault and is useless here.
    ///
    /// ⚠️ **The prekey inside has a lifetime** and this does not check it. A
    /// bundle published in an invite that is scanned months later may carry an
    /// expired prekey; `p2panda-spaces` has `key_bundle_expired` and a rotation
    /// path for exactly this, and nothing here does yet.
    pub fn my_bundle(&self) -> Result<LongTermKeyBundle, Error> {
        let state = self.state.as_ref().ok_or(Error::NoSecret)?;
        KeyManager::prekey_bundle(&state.dcgka.my_keys)
            .map_err(|e| Error::Crypto(e.to_string()))
    }

    /// This vault's key manager, for joining somebody else's group with the
    /// same identity.
    ///
    /// **A FOLLOWER NEEDS ONE IDENTITY AND SEVERAL VAULTS, WHICH IS NOT
    /// OBVIOUS.** A `Vault` holds exactly one group state, so following three
    /// people means three vaults. But the bundle this device published — the
    /// one each subject granted against — belongs to *one* key manager, so all
    /// three have to join using that same manager. A vault that generated its
    /// own would be a different member to the one that was granted, and would
    /// read nothing while looking perfectly healthy.
    ///
    /// ⚠️ **THIS IS SECRET KEY MATERIAL**, not a handle: it is the identity
    /// secret and every prekey secret this device holds. It is exposed because
    /// [`Vault::join`] needs it and the alternative is for each vault to mint
    /// its own, which is the bug above. It must not be logged, written outside
    /// a 0600 vault directory, or sent anywhere.
    pub fn manager_state(&self) -> Result<KeyManagerState, Error> {
        let state = self.state.as_ref().ok_or(Error::NoSecret)?;
        Ok(state.dcgka.my_keys.clone())
    }

    /// A key registry holding the bundles a joiner needs.
    ///
    /// A reader opening its welcome has to be able to encrypt towards whoever
    /// sent it, so it carries the subject's bundle — the same exchange in the
    /// other direction.
    pub fn registry(
        bundles: &[(MemberId, LongTermKeyBundle)],
    ) -> Result<p2panda_encryption::key_registry::KeyRegistryState<MemberId>, Error> {
        let mut registry = KeyRegistry::<MemberId>::init();
        for (id, bundle) in bundles {
            registry = KeyRegistry::add_longterm_bundle(registry, *id, bundle.clone())
                .map_err(|e| Error::Crypto(e.to_string()))?;
        }
        Ok(registry)
    }

    /// Start a group, with this vault as its only member.
    ///
    /// **THE CALLER SUPPLIES THE KEY MANAGER**, and an earlier version did not
    /// — it generated one internally and discarded the bundle it was handed,
    /// so the identity a reader was told about and the identity the subject
    /// actually held were different keys. Nothing noticed while the member id
    /// was a public key, because nothing derived anything from the identity.
    /// The moment a grant tag came from `ECDH(subject, reader)` the two sides
    /// computed different tags and the reader read nothing.
    ///
    /// Readers are added afterwards with [`Vault::grant`], which is where the
    /// relationship tag is derived.
    pub fn create(&mut self, manager: KeyManagerState) -> Result<Message, Error> {
        let dcgka = p2panda_encryption::data_scheme::dcgka::Dcgka::init(
            self.me,
            manager,
            KeyRegistry::<MemberId>::init(),
            Dgm::create(self.me, &[self.me]).expect("infallible"),
        );
        let state = State {
            my_id: self.me,
            dcgka,
            orderer: Default::default(),
            secrets: p2panda_encryption::data_scheme::SecretBundle::init(),
            is_welcomed: false,
        };
        let (state, msg) = Group::create(state, vec![self.me], &self.rng)
            .map_err(|e| Error::Group(e.to_string()))?;
        self.state = Some(state);
        // A subject's own control messages are the ones this vault will accept.
        self.subject_key = Some(self.my_key);
        self.save()?;
        msg.stamp(self.me)
    }

    /// Seal a day into its own segment.
    ///
    /// **UNDER THE LATEST SECRET, NAMING IT.** Not a group message: nothing here
    /// enters the control history, so the cost of sealing does not depend on how
    /// much has been sealed before.
    pub fn seal(&mut self, epoch: i64, records: &[Record]) -> Result<Segment, Error> {
        let path = self.root.join("segments").join(format!("{epoch}.json"));

        // **APPEND, DO NOT REPLACE**, and this used to replace.
        //
        // A caller passes what it has just collected, not the whole day: the
        // AAPS plugin accumulates records between drains and flushes them on a
        // cadence, so one epoch is written to dozens of times as it happens.
        // `fs::write` over the segment destroyed everything sealed into it
        // earlier — silently, from the only copy the subject has, with a reader
        // who could read it yesterday unable to today and no error anywhere.
        //
        // `diaswarm-core`'s seal carries the same comment and the incident that
        // produced it: "found by watching a reader's record count fall from
        // 32,150 to 27,874 between two fetches". This is that bug, reintroduced
        // in the vault meant to replace it, and found by asking what shadow
        // mode would have to compare.
        //
        // A segment that will not open is an ERROR, not an empty start. Reading
        // it as "nothing was there" is how one bad decrypt turns into a deleted
        // day.
        let existing: Vec<Record> = match std::fs::read(&path) {
            Ok(bytes) => {
                let segment: Segment = serde_json::from_slice(&bytes)?;
                self.open_segment(&segment)?.0
            }
            Err(_) => Vec::new(),
        };

        // Anything already in the segment is skipped. The high-water marks
        // upstream stop a record being drained twice, but a full resync would
        // otherwise append the whole history again.
        let seen: std::collections::HashSet<String> =
            existing.iter().map(|r| r.to_canonical_json()).collect();
        let mut merged = existing;
        for record in records {
            if seen.contains(&record.to_canonical_json()) {
                continue;
            }
            merged.push(record.clone());
        }

        let state = self.state.as_ref().ok_or(Error::NoSecret)?;
        // **UNDER THE LATEST SECRET, NOT THE ONE THE SEGMENT ALREADY NAMED**,
        // and the difference is a revocation being real.
        //
        // Reusing the secret already on disk would be the cheaper merge and
        // would mean a reader revoked at noon kept reading the rest of that day
        // — the segment they can already open goes on being appended to. The
        // whole accumulated day is therefore re-sealed under the current
        // secret, so a revocation bites from the moment it happens rather than
        // from midnight.
        //
        // The price, stated rather than discovered: a reader loses the part of
        // *today* they could previously open, and a reader granted at noon can
        // read back to midnight. Closed days are untouched and keep their own
        // secret, so a revoked reader keeps every day that was already
        // finished — which is what `a_granted_reader_opens_segments_and_a_revoked_one_stops`
        // asserts, and still does.
        let secret = state.secrets.latest().ok_or(Error::NoSecret)?;
        let nonce: XAeadNonce = self.rng.random_array().map_err(|e| Error::Crypto(e.to_string()))?;
        let plaintext = encode_records(&merged).into_bytes();
        let ciphertext = encrypt_data(&plaintext, secret, nonce)
            .map_err(|e| Error::Crypto(e.to_string()))?;
        let segment = Segment { epoch, secret_id: secret.id(), nonce, ciphertext };

        // Written beside and renamed: a seal interrupted half way would
        // otherwise leave a truncated segment, which `open_segment` reports as
        // undecryptable and which is the day gone.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&segment)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(segment)
    }

    /// Open one segment, wherever it came from.
    ///
    /// **SEGMENTS DO NOT ONLY COME FROM DISK.** [`read_reporting`] walks a
    /// directory, but a follower's arrive as operation bodies over
    /// `p2panda-net` and never touch one. Both paths need the same three steps
    /// — find the secret this segment names, decrypt, parse — so they are
    /// here once rather than twice.
    ///
    /// Returns the records and how many lines inside would not parse, which the
    /// caller must not drop: see [`Skipped`].
    ///
    /// [`read_reporting`]: Vault::read_reporting
    pub fn open_segment(&self, segment: &Segment) -> Result<(Vec<Record>, usize), Error> {
        let state = self.state.as_ref().ok_or(Error::NoSecret)?;
        let secret = state.secrets.get(&segment.secret_id).ok_or(Error::NotGranted)?;
        let plain = decrypt_data(&segment.ciphertext, secret, segment.nonce)
            .map_err(|_| Error::Undecryptable)?;
        let mut records = Vec::new();
        let mut unparseable = 0;
        for line in String::from_utf8_lossy(&plain).lines() {
            if line.trim().is_empty() {
                continue;
            }
            match Record::from_json(line) {
                Ok(r) => records.push(r),
                Err(_) => unparseable += 1,
            }
        }
        Ok((records, unparseable))
    }

    /// Everything this vault can open from `from_epoch` onwards.
    ///
    /// **THE PROPERTY THE WHOLE DECISION RESTS ON.** Segment files are named by
    /// epoch, so older ones are skipped before anything is decrypted, and each
    /// is opened with the secret it names. Nothing is walked.
    pub fn read_from(&self, from_epoch: i64) -> Result<BTreeMap<i64, Vec<Record>>, Error> {
        Ok(self.read_reporting(from_epoch)?.0)
    }

    /// The same read, and what it could not open.
    ///
    /// **SILENCE IS THE FAILURE THIS PROJECT KEEPS BEING BITTEN BY.** The first
    /// version of `read_from` dropped three different things on the floor with
    /// a bare `continue`: a segment whose secret this vault does not hold, a
    /// segment that will not decrypt, and a record that will not parse. The
    /// first is access control working; the other two are data loss, and from
    /// the outside all three looked like "fewer days than you expected".
    ///
    /// `diaswarm-spaces::Ingested` returns `refused`, `held` and `panicked` for
    /// exactly this reason, and its own comment says why: "a reader that
    /// silently received four days out of five is precisely the failure this
    /// project keeps being bitten by". This is that, for segments.
    pub fn read_reporting(&self, from_epoch: i64) -> Result<(BTreeMap<i64, Vec<Record>>, Skipped), Error> {
        let _ = self.state.as_ref().ok_or(Error::NoSecret)?;
        let mut out = BTreeMap::new();
        let mut skipped = Skipped::default();
        for entry in std::fs::read_dir(self.root.join("segments"))? {
            let path = entry?.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            let Ok(epoch) = stem.parse::<i64>() else { continue };
            if epoch < from_epoch {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                skipped.unreadable += 1;
                continue;
            };
            let Ok(segment) = serde_json::from_slice::<Segment>(&bytes) else {
                skipped.unreadable += 1;
                continue;
            };
            match self.open_segment(&segment) {
                Ok((records, unparseable)) => {
                    skipped.unparseable += unparseable;
                    out.insert(epoch, records);
                }
                // NOT AN ERROR. A segment sealed under a secret this vault was
                // never given is the access control working — a day from before
                // a grant, or after a revocation.
                Err(Error::NotGranted) => skipped.not_ours += 1,
                Err(Error::Undecryptable) => skipped.undecryptable += 1,
                Err(e) => return Err(e),
            }
        }
        Ok((out, skipped))
    }

    /// Add a reader, named by the tag this relationship derives.
    ///
    /// **THE TAG IS COMPUTED HERE RATHER THAN PASSED IN**, so no caller can
    /// accidentally hand a public key to something that publishes it. The
    /// returned tag is what the subject's own private book should file them
    /// under — the grant itself names nobody.
    ///
    /// ⚠️ **A GRANT REACHES BACK OVER EVERYTHING, AND CANNOT BE ASKED NOT TO.**
    /// `EncryptionGroup::add` hands the joiner `&y.secrets` — the whole secret
    /// bundle — so a reader granted today opens every day the subject still
    /// holds a secret for, including days sealed long before they were granted.
    /// There is no `history` flag and no way to pass a narrower bundle: the
    /// library does not expose one.
    ///
    /// **THAT IS LESS THAN BOTH VAULTS THIS REPLACES.** `diaswarm-core` wraps
    /// per segment, so the subject chooses what a reader can open;
    /// [D20](../../docs/decisions.md)'s spaces grant takes an explicit
    /// `history` boolean.
    ///
    /// **AND IT IS OVER-GRANTING, NOT A FEATURE.** The flagship is a parent
    /// watching a child and it needs *24 hours* — that is what D11 asks for and
    /// what the follower shows. Handing over the whole history to satisfy a
    /// one-day need is more than least privilege allows, and it means a
    /// follower compromised at any point exposes everything the subject ever
    /// sealed rather than the day they were watching. Accepted because nothing
    /// turns on it yet and the alternatives cost more today; see D26.
    ///
    /// Revocation is unaffected and still bites forward: see [`Vault::revoke`]
    /// and `seal`'s note on re-sealing under the latest secret.
    pub fn grant(
        &mut self,
        bundle: LongTermKeyBundle,
        purpose: &str,
    ) -> Result<(Message, GrantTag), Error> {
        let mut state = self.state.take().ok_or(Error::NoSecret)?;
        let tag = Self::tag_for(&state.dcgka.my_keys, &bundle, purpose)?;
        state.dcgka.pki = KeyRegistry::add_longterm_bundle(state.dcgka.pki, tag, bundle)
            .map_err(|e| Error::Crypto(e.to_string()))?;
        let (state, msg) =
            Group::add(state, tag, &self.rng).map_err(|e| Error::Group(e.to_string()))?;
        self.state = Some(state);
        self.save()?;
        Ok((msg.stamp(self.me)?, tag))
    }

    /// Remove a reader. Rotates the secret, so the next segment is not theirs.
    pub fn revoke(&mut self, reader: MemberId) -> Result<Message, Error> {
        let state = self.state.take().ok_or(Error::NoSecret)?;
        let (state, msg) =
            Group::remove(state, reader, &self.rng).map_err(|e| Error::Group(e.to_string()))?;
        self.state = Some(state);
        self.save()?;
        msg.stamp(self.me)
    }

    /// Take in a control message from the subject of this vault's group.
    ///
    /// **IT WILL NOT TAKE A BARE [`Message`].** An [`Authentic`] can only come
    /// from [`wire::open_control`], which has already checked the signature, the
    /// author and the claimed sender. What is left for this to check is the one
    /// thing `open_control` cannot know on its own: that the author is *this*
    /// vault's subject, and not some other perfectly valid one. A follower of
    /// two children holds two vaults, and a grant from one is not a grant from
    /// the other.
    pub fn receive(&mut self, message: &Authentic) -> Result<(), Error> {
        match self.subject_key {
            Some(expected) if expected == message.author => {}
            Some(expected) => {
                return Err(Error::Forged(format!(
                    "signed by {} but this vault follows {}",
                    &message.author.to_hex()[..16],
                    &expected.to_hex()[..16]
                )));
            }
            None => {
                return Err(Error::Forged(
                    "this vault has no group yet, so it follows nobody".to_string(),
                ));
            }
        }
        let state = self.state.take().ok_or(Error::NoSecret)?;
        let (mut state, _out) = Group::receive(state, &message.message)
            .map_err(|e| Error::Group(e.to_string()))?;
        state.orderer.saw(message.message.id());
        self.state = Some(state);
        self.save()?;
        Ok(())
    }

    /// Join a group we have been added to, from the welcome we were sent.
    /// Join a group we have been added to.
    ///
    /// **OUR ID HERE IS THE RELATIONSHIP TAG, NOT OUR OWN.** A reader is known
    /// to each subject by a different name, computed from its own secret and
    /// that subject's published identity key — so it has to adopt the right one
    /// before processing a welcome addressed to it.
    pub fn join(
        &mut self,
        manager: KeyManagerState,
        registry: p2panda_encryption::key_registry::KeyRegistryState<MemberId>,
        subject_bundle: &LongTermKeyBundle,
        purpose: &str,
        welcome: &Authentic,
    ) -> Result<(), Error> {
        let was_me = self.me;
        let was_subject = self.subject_key;
        self.me = Self::tag_for(&manager, subject_bundle, purpose)?;
        // Everything afterwards must come from whoever signed the welcome.
        self.subject_key = Some(welcome.author);
        let dcgka = p2panda_encryption::data_scheme::dcgka::Dcgka::init(
            self.me,
            manager,
            registry,
            Dgm::create(self.me, &[]).expect("infallible"),
        );
        let state = State {
            my_id: self.me,
            dcgka,
            orderer: Default::default(),
            secrets: p2panda_encryption::data_scheme::SecretBundle::init(),
            is_welcomed: false,
        };
        let received = Group::receive(state, &welcome.message);
        let (mut state, _out) = match received {
            Ok(pair) => pair,
            Err(e) => {
                self.me = was_me;
                self.subject_key = was_subject;
                return Err(Error::Group(e.to_string()));
            }
        };

        // **A CONTROL MESSAGE THAT IS NOT OUR WELCOME MUST NOT LOOK LIKE ONE.**
        //
        // A subject's control log holds every message it ever published: the
        // group's creation, and a welcome per grant, each encrypted towards a
        // different tag. A reader has to find its own, and nothing in a message
        // says in clear who it is for — a grant that announced its recipient
        // would undo D13. So a reader tries them, which only works if trying
        // the wrong one *fails*.
        //
        // It did not. `Group::receive` returns `Ok` for a `Create` this vault
        // is not in: there is nothing malformed about it, it simply welcomes
        // nobody. The vault was left with a subject, a member id and no
        // secrets, and the first read said `NotGranted` — which reads like a
        // revoked reader rather than a join that never happened.
        if !state.is_welcomed {
            self.me = was_me;
            self.subject_key = was_subject;
            return Err(Error::NotWelcomed);
        }

        state.orderer.saw(welcome.message.id());
        self.state = Some(state);
        self.save()?;
        Ok(())
    }

    /// Whether this vault has been welcomed into its group.
    ///
    /// False for a vault that has never joined, and for one whose `join`
    /// failed. A subject is welcomed by creating.
    pub fn is_welcomed(&self) -> bool {
        self.state.as_ref().map(|s| s.is_welcomed).unwrap_or(false)
    }

    /// How many group secrets this vault holds — the welcome's size, in effect.
    pub fn secrets(&self) -> usize {
        self.state.as_ref().map(|s| s.secrets.len()).unwrap_or(0)
    }
}



/// The group state, written out field by field.
///
/// **BECAUSE `GroupState` CANNOT BE SERIALISED, DESPITE SAYING IT CAN.** Both it
/// and `DcgkaState` are documented "Serializable state ... (for persistence)"
/// and both derive `Serialize`/`Deserialize` — but serde's derive puts the
/// bounds on the *marker* type parameters rather than on their `::State`
/// associated types, and upstream's markers (`KeyManager`, `KeyRegistry<ID>`)
/// derive only `Clone, Debug`. So `serde_json::to_vec(&group_state)` does not
/// compile with production types.
///
/// Every field of `DcgkaState` is public and every one is a concrete
/// serialisable type, so the way through is to take it apart and put it back
/// together. That is what this is. It is a workaround for an upstream defect,
/// not a design, and it should be deleted the day those markers gain a derive.
///
/// In the same family as the three `test_utils`-only escapes
/// [D20](../../docs/decisions.md) had to document for `p2panda-spaces`: the
/// public API returning state the public API cannot store.
#[derive(Deserialize)]
struct Persisted {
    my_id: MemberId,
    /// Whose control messages this vault accepts. `default` so a vault written
    /// before this field existed still opens — as one that follows nobody,
    /// which is the safe reading of "we did not record it".
    #[serde(default)]
    subject_key: Option<p2panda_core::VerifyingKey>,
    pki: p2panda_encryption::key_registry::KeyRegistryState<MemberId>,
    my_keys: p2panda_encryption::key_manager::KeyManagerState,
    two_party: std::collections::HashMap<
        MemberId,
        p2panda_encryption::two_party::TwoPartyState<LongTermKeyBundle>,
    >,
    dgm: group::DgmState,
    orderer: group::OrderState,
    secrets: p2panda_encryption::data_scheme::SecretBundleState,
    is_welcomed: bool,
}

/// **WRITTEN BY REFERENCE, BECAUSE HALF OF IT CANNOT BE CLONED.**
/// `TwoPartyState` and `SecretBundleState` are `Clone` only under `test_utils`,
/// so an owned snapshot is not available to a production build. Borrowing costs
/// nothing and is the only option.
#[derive(Serialize)]
struct PersistedRef<'a> {
    my_id: &'a MemberId,
    subject_key: Option<&'a p2panda_core::VerifyingKey>,
    pki: &'a p2panda_encryption::key_registry::KeyRegistryState<MemberId>,
    my_keys: &'a p2panda_encryption::key_manager::KeyManagerState,
    two_party: &'a std::collections::HashMap<
        MemberId,
        p2panda_encryption::two_party::TwoPartyState<LongTermKeyBundle>,
    >,
    dgm: &'a group::DgmState,
    orderer: &'a group::OrderState,
    secrets: &'a p2panda_encryption::data_scheme::SecretBundleState,
    is_welcomed: bool,
}

impl<'a> PersistedRef<'a> {
    fn of(y: &'a State, subject_key: Option<&'a p2panda_core::VerifyingKey>) -> Self {
        PersistedRef {
            my_id: &y.my_id,
            subject_key,
            pki: &y.dcgka.pki,
            my_keys: &y.dcgka.my_keys,
            two_party: &y.dcgka.two_party,
            dgm: &y.dcgka.dgm,
            orderer: &y.orderer,
            secrets: &y.secrets,
            is_welcomed: y.is_welcomed,
        }
    }
}

impl Persisted {
    fn into_state(self) -> State {
        State {
            my_id: self.my_id,
            dcgka: p2panda_encryption::data_scheme::dcgka::DcgkaState {
                pki: self.pki,
                my_keys: self.my_keys,
                my_id: self.my_id,
                two_party: self.two_party,
                dgm: self.dgm,
            },
            orderer: self.orderer,
            secrets: self.secrets,
            is_welcomed: self.is_welcomed,
        }
    }
}
