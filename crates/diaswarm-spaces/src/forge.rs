//! Turning a spaces message into a p2panda operation, and persisting it.
//!
//! **THE WHOLE INTEGRATION SURFACE IS THIS FILE.** `spike/p2panda-seal` §4
//! predicted the opposite — that supplying the encryption group's four generic
//! parameters for real would be the work, and the cryptography would not be.
//! `p2panda-spaces` supplies all four; what it leaves to the application is one
//! trait with two methods: say who you are, and put a message in a log.
//!
//! It is modelled on `p2panda_spaces::test_utils::TestForge`, which is 71 lines
//! and does the same job for the library's own tests. That is not a shortcut —
//! it is the shape the trait is meant to have. The parts that are ours are the
//! log id and the choice of store.

use p2panda_core::{Hash, Header, SigningKey, VerifyingKey};
use p2panda_spaces::{Forge, SpacesArgs};
use p2panda_store::logs::LogStore;
use p2panda_store::operations::OperationStore;
use p2panda_store::{SqliteError, SqliteStore, tx};

use crate::operation::Inner;
use crate::{Conditions, Operation};

/// One log per subject, and one subject per device.
///
/// A device publishes its own records and nobody else's — replication carries
/// other people's operations, it does not write to their log — so a second log
/// id would have nothing to put in it.
pub const LOG_ID: u32 = 0;

type LogId = u32;
type SeqNum = u32;

#[derive(Debug, Clone)]
pub struct SwarmForge {
    signing_key: SigningKey,
    store: SqliteStore,
}

impl SwarmForge {
    pub fn new(store: SqliteStore, signing_key: SigningKey) -> Self {
        Self { signing_key, store }
    }
}

impl Forge<Conditions> for SwarmForge {
    type Message = Operation;
    type Error = SqliteError;

    fn verifying_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    async fn forge(&self, args: SpacesArgs<Conditions>) -> Result<Self::Message, Self::Error> {
        // THE PAYLOAD GOES IN THE BODY. See `operation.rs` for why: the header
        // is size-limited and an application payload does not belong there.
        // `Builder::body` puts its hash and size into the signed header, so
        // nothing is left unauthenticated by the move.
        let (args, payload) = match args {
            SpacesArgs::Application {
                space_id,
                space_dependencies,
                group_secret_id,
                nonce,
                ciphertext,
            } => (
                SpacesArgs::Application {
                    space_id,
                    space_dependencies,
                    group_secret_id,
                    nonce,
                    ciphertext: Vec::new(),
                },
                Some(ciphertext),
            ),
            other => (other, None),
        };

        // ONE TRANSACTION for read-then-append. Two concurrent forges that both
        // read the same latest entry would build two operations claiming the
        // same seq_num and backlink, which is a forked log — and a forked log
        // is not a recoverable state, it is a subject nobody can replicate.
        let operation = tx!(self.store, {
            let (seq_num, backlink) =
                <SqliteStore as LogStore<Inner, VerifyingKey, LogId, SeqNum, Hash>>::get_latest_entry_tx(
                    &self.store,
                    &self.signing_key.verifying_key(),
                    &LOG_ID,
                )
                .await?
                .map(|op| (op.header.seq_num + 1, Some(op.hash)))
                .unwrap_or((0, None));

            let mut builder = Header::builder().seq_num(seq_num).backlink(backlink);
            if let Some(bytes) = &payload {
                builder = builder.body(bytes);
            }
            let header = builder.build(&self.signing_key, args);

            let operation =
                Inner::from_parts(header, payload.clone().map(p2panda_core::Body::from_bytes));
            self.store.insert_operation(&operation.hash, &operation, &LOG_ID).await?;
            Operation::wrap(operation)
        });

        Ok(operation)
    }
}
