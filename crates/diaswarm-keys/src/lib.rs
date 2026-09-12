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

pub mod group;

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

use group::{Dgm, MemberId, Message, Order, OperationId};

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
    /// Open or create a vault at `root`.
    pub fn open(root: impl AsRef<Path>, offset: i64, signing: &SigningKey) -> Result<Self, Error> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(root.join("segments"))?;
        let rng = Rng::default();
        let me = signing.verifying_key();
        Ok(Vault { root, offset, state: None, rng, me })
    }

    pub fn subject(&self) -> MemberId {
        self.me
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

    /// Start a group with the given members, having registered their bundles.
    pub fn create(
        &mut self,
        signing: &SigningKey,
        members: Vec<(MemberId, LongTermKeyBundle)>,
    ) -> Result<Message, Error> {
        let _ = signing;
        let (manager, _) = Self::key_bundle(&self.rng)?;
        let mut registry = KeyRegistry::<MemberId>::init();
        for (id, bundle) in &members {
            registry = KeyRegistry::add_longterm_bundle(registry, *id, bundle.clone())
                .map_err(|e| Error::Crypto(e.to_string()))?;
        }
        let dcgka = p2panda_encryption::data_scheme::dcgka::Dcgka::init(
            self.me,
            manager,
            registry,
            Dgm::create(self.me, &[self.me]).expect("infallible"),
        );
        let state = State {
            my_id: self.me,
            dcgka,
            orderer: Default::default(),
            secrets: p2panda_encryption::data_scheme::SecretBundle::init(),
            is_welcomed: false,
        };
        let ids: Vec<MemberId> = members.iter().map(|(id, _)| *id).collect();
        let (state, msg) =
            Group::create(state, ids, &self.rng).map_err(|e| Error::Group(e.to_string()))?;
        self.state = Some(state);
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
        let state = self.state.as_ref().ok_or(Error::NoSecret)?;
        let mut out = BTreeMap::new();
        for entry in std::fs::read_dir(self.root.join("segments"))? {
            let path = entry?.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            let Ok(epoch) = stem.parse::<i64>() else { continue };
            if epoch < from_epoch {
                continue;
            }
            let segment: Segment = serde_json::from_slice(&std::fs::read(&path)?)?;
            let Some(secret) = state.secrets.get(&segment.secret_id) else { continue };
            let Ok(plain) = decrypt_data(&segment.ciphertext, secret, segment.nonce) else {
                continue;
            };
            let records: Vec<Record> = String::from_utf8_lossy(&plain)
                .lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| Record::from_json(l).ok())
                .collect();
            out.insert(epoch, records);
        }
        Ok(out)
    }

    /// Add a reader, having registered their key bundle.
    pub fn grant(&mut self, reader: MemberId, bundle: LongTermKeyBundle) -> Result<Message, Error> {
        let mut state = self.state.take().ok_or(Error::NoSecret)?;
        state.dcgka.pki = KeyRegistry::add_longterm_bundle(state.dcgka.pki, reader, bundle)
            .map_err(|e| Error::Crypto(e.to_string()))?;
        let (state, msg) =
            Group::add(state, reader, &self.rng).map_err(|e| Error::Group(e.to_string()))?;
        self.state = Some(state);
        Ok(msg.stamp(self.me))
    }

    /// Remove a reader. Rotates the secret, so the next segment is not theirs.
    pub fn revoke(&mut self, reader: MemberId) -> Result<Message, Error> {
        let state = self.state.take().ok_or(Error::NoSecret)?;
        let (state, msg) =
            Group::remove(state, reader, &self.rng).map_err(|e| Error::Group(e.to_string()))?;
        self.state = Some(state);
        Ok(msg.stamp(self.me))
    }

    /// Take in a control message from somebody else.
    pub fn receive(&mut self, message: Message) -> Result<(), Error> {
        let state = self.state.take().ok_or(Error::NoSecret)?;
        let (mut state, _out) =
            Group::receive(state, &message).map_err(|e| Error::Group(e.to_string()))?;
        state.orderer.saw(message.id());
        self.state = Some(state);
        Ok(())
    }

    /// Join a group we have been added to, from the welcome we were sent.
    pub fn join(
        &mut self,
        signing: &SigningKey,
        manager: KeyManagerState,
        registry: p2panda_encryption::key_registry::KeyRegistryState<MemberId>,
        welcome: Message,
    ) -> Result<(), Error> {
        let _ = signing;
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
        Ok(())
    }

    /// How many group secrets this vault holds — the welcome's size, in effect.
    pub fn secrets(&self) -> usize {
        self.state.as_ref().map(|s| s.secrets.len()).unwrap_or(0)
    }
}


