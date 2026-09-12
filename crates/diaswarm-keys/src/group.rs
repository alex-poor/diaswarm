//! The two traits `p2panda-encryption` needs and `p2panda-spaces` keeps private.
//!
//! **242 LINES OF BOOKKEEPING, WHICH IS THE WHOLE ARGUMENT FOR D26.** Using the
//! encryption layer directly means supplying a `GroupMembership` and an
//! `Ordering`. `p2panda-spaces` has both and exports neither — they are
//! `pub(crate)`. Reading them first was the condition on the decision: if they
//! held judgement about concurrent membership, reimplementing them would be
//! bespoke security code and the trade would not be worth making.
//!
//! They do not. Upstream's `EncryptionGroupMembership` is a declared
//! placeholder — "most methods perform no actual actions as group management is
//! handled by p2panda-auth" — and its `EncryptionOrderer` says "it does not take
//! care of ordering of control and application messages". Both are `Infallible`
//! throughout. What is here is the same shape: a set of members, a dependency
//! graph for stamping heads, and a queue that waits until we have been welcomed.
//!
//! **AND THE APPLICATION HALF IS DEAD CODE ON PURPOSE.** Upstream's orderer
//! carries a TODO — "currently application messages are also included in the
//! dependency graph, we want to separate these from control messages eventually
//! in order to support pruning". Under D26 records never become group messages
//! at all: they are sealed into segments with `encrypt_data`. So this graph only
//! ever holds control messages, one per grant or rotation rather than one per
//! reading, and the thing that TODO is about cannot arise.

use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::Infallible;

use p2panda_core::{Hash, VerifyingKey};
use p2panda_encryption::crypto::xchacha20::XAeadNonce;
use p2panda_encryption::data_scheme::{ControlMessage, DirectMessage, GroupSecretId};
use p2panda_encryption::traits::{GroupMembership, GroupMessage, GroupMessageContent, Ordering};
use serde::{Deserialize, Serialize};

/// **A MEMBER IS NAMED BY A PER-RELATIONSHIP TAG, NOT BY THEIR KEY.**
/// `ControlMessage::Add { added: ID }` publishes this identifier in clear, so
/// what it is decides whether a grant leaks the social graph. A public key is
/// the same in every subject's log and would cluster a clinician's patients —
/// [D13](../../docs/decisions.md) removed that and this must not put it back.
pub type MemberId = GrantTag;
pub type OperationId = Hash;

/// A per-relationship name for a member, so a control message names nobody.
///
/// `HKDF(ECDH(subject, reader), "diaswarm-grant-tag-v1" || purpose)` — D13's
/// construction, and deliberately the same bytes: `diaswarm-core` and this
/// crate derive it through `grant_tag_from_shared`, so one relationship has one
/// identifier whichever vault wrote it.
///
/// It works as a group member id because `IdentityHandle` requires only
/// `Copy + Debug + PartialEq + Eq + Hash`, is not sealed, and the 2SM key
/// agreement uses the key *bundle* and never the handle. The handle is a label
/// the application picks.
///
/// **Unlinkable across subjects**, because the shared secret differs per pair.
/// The reader computes the same tag from their own side, so nobody has to be
/// told what they are called. What still leaks is what D13 says still leaks:
/// how many grant events there have been, and roughly when.
#[derive(Copy, Clone, Debug, PartialEq, Eq, std::hash::Hash, Serialize, Deserialize)]
pub struct GrantTag(pub [u8; 32]);

impl p2panda_encryption::traits::IdentityHandle for GrantTag {}

impl std::fmt::Display for GrantTag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for b in &self.0[..8] {
            write!(f, "{b:02x}")?;
        }
        write!(f, "…")
    }
}

impl GrantTag {
    /// The tag a subject uses for itself inside its own group.
    ///
    /// **DELIBERATELY NOT UNLINKABLE, BECAUSE THERE IS NOTHING TO HIDE.** A
    /// grant tag conceals *who a reader is*, and the subject of a log is
    /// already named by the log — its key is in the invite anybody scanned. So
    /// this is a plain function of the subject's own key: derivable by anyone,
    /// which costs nothing, and stable across restarts, which a random one
    /// would not be.
    pub fn own(key: &VerifyingKey) -> Self {
        let mut input = Vec::with_capacity(64);
        input.extend_from_slice(b"diaswarm-self-v1");
        input.extend_from_slice(key.as_bytes());
        GrantTag(*Hash::digest(&input).as_bytes())
    }
}

/// Who is in the group.
///
/// A set, and nothing else. Membership decisions are made where they belong —
/// in the grant, by the subject — and this records the outcome so the key
/// agreement knows who to encrypt a welcome towards.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dgm;

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct DgmState {
    pub members: HashSet<MemberId>,
}

impl GroupMembership<MemberId, OperationId> for Dgm {
    type State = DgmState;
    type Error = Infallible;

    fn create(_me: MemberId, initial: &[MemberId]) -> Result<Self::State, Self::Error> {
        Ok(DgmState { members: initial.iter().cloned().collect() })
    }

    fn from_welcome(_me: MemberId, y: Self::State) -> Result<Self::State, Self::Error> {
        Ok(y)
    }

    fn add(
        mut y: Self::State,
        _adder: MemberId,
        added: MemberId,
        _op: OperationId,
    ) -> Result<Self::State, Self::Error> {
        y.members.insert(added);
        Ok(y)
    }

    fn remove(
        mut y: Self::State,
        _remover: MemberId,
        removed: &MemberId,
        _op: OperationId,
    ) -> Result<Self::State, Self::Error> {
        y.members.remove(removed);
        Ok(y)
    }

    fn members(y: &Self::State) -> Result<HashSet<MemberId>, Self::Error> {
        Ok(y.members.clone())
    }
}

/// A group message: a control message and whatever direct messages ride with it.
///
/// The application variant exists because the trait requires it and is never
/// constructed — see the module note.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Message {
    Control {
        id: OperationId,
        sender: MemberId,
        dependencies: Vec<OperationId>,
        control: ControlMessage<MemberId>,
        direct: Vec<DirectMessage<MemberId, OperationId, Dgm>>,
    },
    Application {
        id: OperationId,
        sender: MemberId,
        dependencies: Vec<OperationId>,
        secret_id: GroupSecretId,
        nonce: XAeadNonce,
        ciphertext: Vec<u8>,
    },
}

impl Message {
    pub fn id(&self) -> OperationId {
        match self {
            Message::Control { id, .. } | Message::Application { id, .. } => *id,
        }
    }

    pub fn dependencies(&self) -> &[OperationId] {
        match self {
            Message::Control { dependencies, .. } | Message::Application { dependencies, .. } => {
                dependencies
            }
        }
    }

    /// Who this message says it is from.
    ///
    /// A claim until [`crate::wire::open_control`] has checked it against a
    /// signature — which is why [`crate::Vault::receive`] will not take a bare
    /// `Message`.
    pub fn sender(&self) -> MemberId {
        match self {
            Message::Control { sender, .. } | Message::Application { sender, .. } => *sender,
        }
    }

    /// Stamp the sender and a content-derived id, which the orderer cannot do
    /// because it does not hold the identity.
    ///
    /// **THE SENDER GOES ON BEFORE THE HASH, AND THE FIRST VERSION DID IT THE
    /// OTHER WAY.** Hashing first meant the id did not commit to who sent it, so
    /// two members publishing structurally identical control messages — two
    /// subjects each creating a group, say — derived the *same* id. The
    /// orderer keys its `messages` map and its seen-set on that id, so one
    /// would have silently displaced the other.
    pub fn stamp(mut self, sender: MemberId) -> Result<Self, crate::Error> {
        match &mut self {
            Message::Control { sender: s, .. } | Message::Application { sender: s, .. } => {
                *s = sender;
            }
        }
        let id = self.content_id()?;
        match &mut self {
            Message::Control { id: i, .. } | Message::Application { id: i, .. } => *i = id,
        }
        Ok(self)
    }

    /// The id: a hash over the sender, the dependencies and the content.
    ///
    /// **CBOR, AND THE ERROR IS NOT SWALLOWED — BOTH WERE WRONG BEFORE.** This
    /// was `serde_json::to_vec(&self).unwrap_or_default()`, which has two
    /// faults that compound. JSON cannot encode a map with a non-string key,
    /// and a `DirectMessage` carries exactly that — the same wall
    /// [`crate::Vault::save`] hit and switched to CBOR for. And
    /// `unwrap_or_default()` turns that failure into empty bytes, so *every*
    /// message would have come out with `Hash::digest(b"")` as its id: one id
    /// for the whole group, silently, with the orderer keying its map and its
    /// seen-set on it.
    ///
    /// **THE ID FIELD IS EXCLUDED RATHER THAN ZEROED**, so the hash is over a
    /// shape that has no id in it at all. Hashing `self` with a placeholder id
    /// would have made the result depend on which placeholder the orderer
    /// happened to use, which is not a property anything should rest on.
    pub fn content_id(&self) -> Result<OperationId, crate::Error> {
        let canonical = match self {
            Message::Control { sender, dependencies, control, direct, .. } => {
                Canonical::Control { sender, dependencies, control, direct }
            }
            Message::Application { sender, dependencies, secret_id, nonce, ciphertext, .. } => {
                Canonical::Application { sender, dependencies, secret_id, nonce, ciphertext }
            }
        };
        let bytes = p2panda_core::cbor::encode_cbor(&canonical)
            .map_err(|e| crate::Error::Encode(e.to_string()))?;
        Ok(Hash::digest(&bytes))
    }
}

/// [`Message`] without its id, which is what the id is a hash of.
#[derive(Serialize)]
enum Canonical<'a> {
    Control {
        sender: &'a MemberId,
        dependencies: &'a [OperationId],
        control: &'a ControlMessage<MemberId>,
        direct: &'a [DirectMessage<MemberId, OperationId, Dgm>],
    },
    Application {
        sender: &'a MemberId,
        dependencies: &'a [OperationId],
        secret_id: &'a GroupSecretId,
        nonce: &'a XAeadNonce,
        ciphertext: &'a [u8],
    },
}

impl GroupMessage<MemberId, OperationId, Dgm> for Message {
    fn id(&self) -> OperationId {
        Message::id(self)
    }

    fn sender(&self) -> MemberId {
        Message::sender(self)
    }

    fn content(&self) -> GroupMessageContent<MemberId> {
        match self {
            Message::Control { control, .. } => GroupMessageContent::Control(control.clone()),
            Message::Application { ciphertext, nonce, secret_id, .. } => {
                GroupMessageContent::Application {
                    ciphertext: ciphertext.clone(),
                    nonce: *nonce,
                    group_secret_id: *secret_id,
                }
            }
        }
    }

    fn direct_messages(&self) -> Vec<DirectMessage<MemberId, OperationId, Dgm>> {
        match self {
            Message::Control { direct, .. } => direct.clone(),
            Message::Application { .. } => Vec::new(),
        }
    }
}

/// Heads for stamping, and a queue that waits to be welcomed.
///
/// Causal ordering is deliberately not here — upstream's equivalent says the
/// same of itself. It belongs to whoever delivers messages, which in this
/// project is `p2panda-store`'s `OrdererStore`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Order;

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct OrderState {
    heads: Vec<OperationId>,
    seen: HashSet<OperationId>,
    queue: VecDeque<OperationId>,
    messages: HashMap<OperationId, Message>,
    welcomed: bool,
}

impl OrderState {
    pub fn heads(&self) -> Vec<OperationId> {
        self.heads.clone()
    }

    /// Record a message and make it the new head.
    ///
    /// A head list rather than a graph: with one writer per group — the
    /// subject — the control history is a line, and the tips of a line are its
    /// last element. Upstream keeps a `DiGraphMap` because a space may have
    /// several writers of control messages; here only the subject grants.
    pub fn saw(&mut self, id: OperationId) {
        if self.seen.insert(id) {
            self.heads = vec![id];
        }
    }
}

impl Ordering<MemberId, OperationId, Dgm> for Order {
    type State = OrderState;
    type Error = Infallible;
    type Message = Message;

    fn next_control_message(
        y: Self::State,
        control: &ControlMessage<MemberId>,
        direct: &[DirectMessage<MemberId, OperationId, Dgm>],
    ) -> Result<(Self::State, Self::Message), Self::Error> {
        let dependencies = y.heads();
        Ok((
            y,
            Message::Control {
                id: Hash::digest(b""),
                sender: dummy_member(),
                dependencies,
                control: control.clone(),
                direct: direct.to_vec(),
            },
        ))
    }

    fn next_application_message(
        y: Self::State,
        secret_id: GroupSecretId,
        nonce: XAeadNonce,
        ciphertext: Vec<u8>,
    ) -> Result<(Self::State, Self::Message), Self::Error> {
        let dependencies = y.heads();
        Ok((
            y,
            Message::Application {
                id: Hash::digest(b""),
                sender: dummy_member(),
                dependencies,
                secret_id,
                nonce,
                ciphertext,
            },
        ))
    }

    fn queue(mut y: Self::State, message: &Self::Message) -> Result<Self::State, Self::Error> {
        let id = message.id();
        y.messages.insert(id, message.clone());
        y.queue.push_back(id);
        Ok(y)
    }

    fn set_welcome(mut y: Self::State, message: &Self::Message) -> Result<Self::State, Self::Error> {
        y.welcomed = true;
        y.saw(message.id());
        Ok(y)
    }

    fn next_ready_message(
        mut y: Self::State,
    ) -> Result<(Self::State, Option<Self::Message>), Self::Error> {
        if !y.welcomed {
            return Ok((y, None));
        }
        let next = y.queue.pop_front().and_then(|id| y.messages.get(&id).cloned());
        Ok((y, next))
    }
}

/// A placeholder sender, replaced by [`Message::stamp`] before the message
/// leaves the vault. The trait hands the orderer no identity to use.
fn dummy_member() -> MemberId {
    GrantTag([0u8; 32])
}
