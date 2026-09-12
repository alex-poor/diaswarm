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
//! Auth — who may grant, and the tamper-evident record of grants ([D13]) — is a
//! separate question this crate does not answer.
//!
//! 🔴 **CONTROL MESSAGES ARE NOT AUTHENTICATED, AND NOTHING HERE CARRIES THEM
//! YET.** [`group::Message`] is a plain struct with a `sender` field anybody can
//! set, and [`Vault::receive`] processes one without checking who sent it.
//! `p2panda-spaces` avoids this by making every message a signed
//! `p2panda_core::Operation`; [`wire`] does that for *segments* and not for
//! control messages, so there is currently no transport for a grant at all.
//!
//! That makes it incomplete rather than exploitable as it stands — a forged
//! message has to reach `receive`, and nothing delivers one — but it is the
//! next thing to fix and it must be fixed before anything replicates grants.
//! The shape is the same as `wire::publish`: put the control message in an
//! operation, let the signature say who sent it, and check it against the
//! member the message claims to be from.

pub mod group;
pub mod wire;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use p2panda_core::SigningKey;
use p2panda_encryption::Rng;
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

pub struct Vault {
    root: PathBuf,
    offset: i64,
    state: Option<State>,
    rng: Rng,
    me: MemberId,
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
        let state = Self::load(&root)?;
        Ok(Vault { root, offset, state, rng, me })
    }

    /// Where the group state lives. One file, beside the segments it opens.
    fn state_path(root: &Path) -> PathBuf {
        root.join("group.cbor")
    }

    fn load(root: &Path) -> Result<Option<State>, Error> {
        let path = Self::state_path(root);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path)?;
        let saved: Persisted = p2panda_core::cbor::decode_cbor(&bytes[..])
            .map_err(|e| Error::Crypto(format!("state will not decode: {e}")))?;
        Ok(Some(saved.into_state()))
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
        let encoded = p2panda_core::cbor::encode_cbor(&PersistedRef::of(state))
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
        self.save()?;
        Ok(msg.stamp(self.me))
    }

    /// Seal a day into its own segment.
    ///
    /// **UNDER THE LATEST SECRET, NAMING IT.** Not a group message: nothing here
    /// enters the control history, so the cost of sealing does not depend on how
    /// much has been sealed before.
    pub fn seal(&mut self, epoch: i64, records: &[Record]) -> Result<Segment, Error> {
        let state = self.state.as_ref().ok_or(Error::NoSecret)?;
        let secret = state.secrets.latest().ok_or(Error::NoSecret)?;
        let nonce: XAeadNonce = self.rng.random_array().map_err(|e| Error::Crypto(e.to_string()))?;
        let plaintext = encode_records(records).into_bytes();
        let ciphertext = encrypt_data(&plaintext, secret, nonce)
            .map_err(|e| Error::Crypto(e.to_string()))?;
        let segment = Segment { epoch, secret_id: secret.id(), nonce, ciphertext };
        let path = self.root.join("segments").join(format!("{epoch}.json"));
        std::fs::write(path, serde_json::to_vec(&segment)?)?;
        Ok(segment)
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
        let state = self.state.as_ref().ok_or(Error::NoSecret)?;
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
            // NOT AN ERROR. A segment sealed under a secret this vault was never
            // given is the access control working — a day from before a grant,
            // or after a revocation.
            let Some(secret) = state.secrets.get(&segment.secret_id) else {
                skipped.not_ours += 1;
                continue;
            };
            let Ok(plain) = decrypt_data(&segment.ciphertext, secret, segment.nonce) else {
                skipped.undecryptable += 1;
                continue;
            };
            let mut records = Vec::new();
            for line in String::from_utf8_lossy(&plain).lines() {
                if line.trim().is_empty() {
                    continue;
                }
                match Record::from_json(line) {
                    Ok(r) => records.push(r),
                    Err(_) => skipped.unparseable += 1,
                }
            }
            out.insert(epoch, records);
        }
        Ok((out, skipped))
    }

    /// Add a reader, named by the tag this relationship derives.
    ///
    /// **THE TAG IS COMPUTED HERE RATHER THAN PASSED IN**, so no caller can
    /// accidentally hand a public key to something that publishes it. The
    /// returned tag is what the subject's own private book should file them
    /// under — the grant itself names nobody.
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
        Ok((msg.stamp(self.me), tag))
    }

    /// Remove a reader. Rotates the secret, so the next segment is not theirs.
    pub fn revoke(&mut self, reader: MemberId) -> Result<Message, Error> {
        let state = self.state.take().ok_or(Error::NoSecret)?;
        let (state, msg) =
            Group::remove(state, reader, &self.rng).map_err(|e| Error::Group(e.to_string()))?;
        self.state = Some(state);
        self.save()?;
        Ok(msg.stamp(self.me))
    }

    /// Take in a control message from somebody else.
    pub fn receive(&mut self, message: Message) -> Result<(), Error> {
        let state = self.state.take().ok_or(Error::NoSecret)?;
        let (mut state, _out) =
            Group::receive(state, &message).map_err(|e| Error::Group(e.to_string()))?;
        state.orderer.saw(message.id());
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
        welcome: Message,
    ) -> Result<(), Error> {
        self.me = Self::tag_for(&manager, subject_bundle, purpose)?;
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
        let (mut state, _out) =
            Group::receive(state, &welcome).map_err(|e| Error::Group(e.to_string()))?;
        state.orderer.saw(welcome.id());
        self.state = Some(state);
        self.save()?;
        Ok(())
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
    fn of(y: &'a State) -> Self {
        PersistedRef {
            my_id: &y.my_id,
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
