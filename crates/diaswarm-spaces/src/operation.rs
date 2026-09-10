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
//!
//! IT ALSO CARRIES THE PAYLOAD IN THE RIGHT PLACE. `p2panda-spaces` puts an
//! application message's ciphertext inline in `SpacesArgs::Application`, which
//! rides in the operation's *header* — and `p2panda-core` decodes headers with
//! `.length_limit(512)`. Measured, that caps a published payload at well under
//! 2 KB, which turned 2.71 MB of records into 10,569 operations, 3x inflation
//! on the wire, and a quadratic read that took 27 minutes.
//!
//! An operation has a **body** for exactly this, and `Builder::body()` folds its
//! hash and size into the signed header. So [`SwarmForge`](crate::SwarmForge)
//! moves the ciphertext there when it forges, and this type puts it back when
//! `p2panda-spaces` asks for the args — which is what `Forge` is for: its own
//! documentation calls it the "interface for wrapping forge args in custom
//! message types".
//!
//! Nothing about the encryption changes. The bytes are identical and still
//! produced and consumed entirely by the library; only where they sit inside
//! the envelope is ours to choose.

use std::borrow::Borrow;

use p2panda_core::traits::{Digest, Provenance};
use p2panda_core::{Hash, VerifyingKey};
use p2panda_spaces::SpacesArgs;

use crate::Conditions;

/// What actually goes in a log and over the wire.
pub type Inner = p2panda_core::Operation<SpacesArgs<Conditions>>;

// Equality is the operation's, not the restored args': two operations with the
// same hash are the same operation, and `SpacesArgs` is not `Eq` anyway.
#[derive(Debug, Clone)]
pub struct Operation {
    inner: Inner,
    /// The args as `p2panda-spaces` published them, with the ciphertext put
    /// back from the body. Held rather than computed because `Borrow` returns a
    /// reference.
    args: SpacesArgs<Conditions>,
}

impl PartialEq for Operation {
    fn eq(&self, other: &Self) -> bool {
        self.inner.hash == other.inner.hash
    }
}

impl Eq for Operation {}

impl Operation {
    /// Wrap an operation, restoring the args the spaces layer expects.
    pub fn wrap(inner: Inner) -> Self {
        let args = restore(&inner);
        Operation { inner, args }
    }

    pub fn hash(&self) -> Hash {
        self.inner.hash
    }

    pub fn inner(&self) -> &Inner {
        &self.inner
    }
}

/// Put an application message's ciphertext back where the library expects it.
///
/// The body is the authority whenever there is one: the forge always moves an
/// `Application` payload there, so a header carrying an empty ciphertext beside
/// a body is the normal case rather than a damaged one. Anything else is passed
/// through untouched — auth and membership messages are small and stay in the
/// header, where their dependencies are read without fetching a body.
fn restore(inner: &Inner) -> SpacesArgs<Conditions> {
    match (&inner.header.extensions, &inner.body) {
        (
            SpacesArgs::Application {
                space_id,
                space_dependencies,
                group_secret_id,
                nonce,
                ciphertext,
            },
            Some(body),
        ) if ciphertext.is_empty() => SpacesArgs::Application {
            space_id: *space_id,
            space_dependencies: space_dependencies.clone(),
            group_secret_id: *group_secret_id,
            nonce: *nonce,
            ciphertext: body.to_bytes(),
        },
        (args, _) => args.clone(),
    }
}

impl From<Inner> for Operation {
    fn from(inner: Inner) -> Self {
        Operation::wrap(inner)
    }
}

impl Borrow<SpacesArgs<Conditions>> for Operation {
    fn borrow(&self) -> &SpacesArgs<Conditions> {
        &self.args
    }
}

impl Provenance<VerifyingKey> for Operation {
    fn author(&self) -> VerifyingKey {
        self.inner.author()
    }

    fn verify(&self) -> bool {
        self.inner.verify()
    }
}

impl Digest<Hash> for Operation {
    fn hash(&self) -> Hash {
        self.inner.hash
    }
}
