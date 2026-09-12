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

pub type MemberId = VerifyingKey;
pub type OperationId = Hash;

/// Who is in the group.
///
/// A set, and nothing else. Membership decisions are made where they belong —
/// in the grant, by the subject — and this records the outcome so the key
/// agreement knows who to encrypt a welcome towards.
#[derive(Clone, Debug)]
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

    /// Stamp the sender and a content-derived id, which the orderer cannot do
    /// because it does not hold the identity.
    pub fn stamp(mut self, sender: MemberId) -> Self {
        let bytes = serde_json::to_vec(&self).unwrap_or_default();
        let id = Hash::digest(&bytes);
        match &mut self {
            Message::Control { id: i, sender: s, .. } => {
                *i = id;
                *s = sender;
            }
            Message::Application { id: i, sender: s, .. } => {
                *i = id;
                *s = sender;
            }
        }
        self
    }
}

impl GroupMessage<MemberId, OperationId, Dgm> for Message {
    fn id(&self) -> OperationId {
        Message::id(self)
    }

    fn sender(&self) -> MemberId {
        match self {
            Message::Control { sender, .. } | Message::Application { sender, .. } => *sender,
        }
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
#[derive(Clone, Debug)]
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
    p2panda_core::SigningKey::from_bytes(&[1u8; 32]).verifying_key()
}
