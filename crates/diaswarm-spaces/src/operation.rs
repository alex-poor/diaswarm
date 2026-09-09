//! A p2panda operation, wrapped so this crate can speak for it.
//!
//! WHY A NEWTYPE AND NOT AN ALIAS. `Forge::Message` must implement
//! `Borrow<SpacesArgs<C>>` — the spaces layer reads a message's arguments back
//! out of whatever type the application publishes. `p2panda_spaces` writes that
//! impl for its own test operation because `SpacesArgs` is its own type; from
//! out here both `Operation` and `SpacesArgs` are foreign, and the orphan rule
//! says no.
//!
//! So the wrapper is not ceremony, it is the only way to attach the impl. It
//! costs three forwarding methods and buys the whole spaces stack.

use std::borrow::Borrow;

use p2panda_core::traits::{Digest, Provenance};
use p2panda_core::{Hash, VerifyingKey};
use p2panda_spaces::SpacesArgs;

use crate::Conditions;

/// What actually goes in a log and over the wire.
pub type Inner = p2panda_core::Operation<SpacesArgs<Conditions>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operation(pub Inner);

impl Operation {
    pub fn hash(&self) -> Hash {
        self.0.hash
    }

    pub fn inner(&self) -> &Inner {
        &self.0
    }
}

impl From<Inner> for Operation {
    fn from(inner: Inner) -> Self {
        Operation(inner)
    }
}

impl Borrow<SpacesArgs<Conditions>> for Operation {
    fn borrow(&self) -> &SpacesArgs<Conditions> {
        &self.0.header.extensions
    }
}

impl Provenance<VerifyingKey> for Operation {
    fn author(&self) -> VerifyingKey {
        self.0.author()
    }

    fn verify(&self) -> bool {
        self.0.verify()
    }
}

impl Digest<Hash> for Operation {
    fn hash(&self) -> Hash {
        self.0.hash
    }
}
