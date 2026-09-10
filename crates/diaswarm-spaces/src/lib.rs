//! The vault, on p2panda's own layers.
//!
//! WHAT THIS REPLACES, AND WHY IT IS A SEPARATE CRATE. `diaswarm-core`'s
//! [`vault`](../diaswarm_core/vault/index.html) is 933 lines of hand-composed
//! cryptography — a content key per segment, that key wrapped separately to
//! every reader, and a signed hash-chained grant log filed under unlinkable
//! tags. It works and it is tested. Nobody qualified has reviewed it, and
//! SECURITY.md leads with that.
//!
//! `spike/p2panda-spaces` measured the alternative: every grant this project
//! offers is a method call on a `Space`. Granting with history is `add`,
//! granting from now on is a new space, revoking is `remove`, and a peer that
//! holds ciphertext it cannot read falls out of the design rather than being
//! engineered into it.
//!
//! **It is a separate crate so both exist at once.** `diaswarm-core` stays as
//! it is, and `tests/differential.rs` puts identical records through both and
//! compares what comes out. Deleting the hand-rolled path before that agrees
//! would mean trusting a rewrite of the one component whose failure mode is
//! somebody's glucose history being readable by the wrong person.
//!
//! WHAT A WINDOW IS. A space. Sealing publishes into the current window; a
//! reader granted "from now on" is added to a new window and is simply not a
//! member of the space holding the earlier days, so there is nothing to
//! withhold. A reader granted "everything" is added to every window that
//! exists. The subject's own bookkeeping is therefore one integer — how many
//! windows there are — and space ids are derived from it, so a reader can name
//! a window without being told.

use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use futures::FutureExt;

use p2panda_auth::Access;
use p2panda_core::{Hash, VerifyingKey};
use p2panda_encryption::Rng;
use p2panda_spaces::manager::Manager;
use p2panda_spaces::space::Space;
use p2panda_spaces::{Credentials, Event, SpaceId, SpacesArgs, StrongRemoveResolver};
use p2panda_auth::group::GroupCrdtState;
use p2panda_spaces::space::SpacesState;
use p2panda_spaces::{ActorId, AuthMessage, OperationId, SpacesStoreState};
use p2panda_store::groups::GroupsStore;
use p2panda_store::spaces::{SpacesStore, SqliteSpacesStore};
use p2panda_store::tx;
use p2panda_store::{SqliteStore, SqliteStoreBuilder};

use diaswarm_core::{Record, encode, encode_records};

pub mod forge;
pub mod operation;
pub use forge::{LOG_ID, SwarmForge};
pub use operation::Operation;

/// Access conditions. Unit for now.
///
/// p2panda-auth carries an application-defined condition on every capability,
/// which is where a *purpose* ("follow", "clinic") would live — the thing D13
/// currently encodes by deriving a separate tag per purpose. Left unit until
/// the migration is otherwise complete, so that one change is one change.
pub type Conditions = ();

type Resolver = StrongRemoveResolver<Conditions>;
type AuthState = GroupCrdtState<ActorId, OperationId, AuthMessage<Conditions>, Conditions>;

/// The key the global auth state is filed under.
///
/// **REPRODUCED FROM `p2panda_spaces::manager`, WHERE IT IS PRIVATE.** See
/// [`Vault::persist_group`] for why this crate has to write that state itself.
/// If upstream ever changes the string, state would be written where the
/// manager does not look for it and every grant would silently stop
/// persisting — so `state_survives_a_reopen` in `tests/vault.rs` round-trips
/// through the manager's own public read path, and fails loudly if it drifts.
const GLOBAL_GROUPS_CONTEXT_ID: &[u8] = b"global-groups-context";
type Store = SqliteSpacesStore<SpacesArgs<Conditions>>;
type SwarmManager = Manager<Store, SwarmForge, Conditions, Resolver>;
type SwarmSpace = Space<Store, SwarmForge, Conditions, Resolver>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("store: {0}")]
    Store(String),
    #[error("spaces: {0}")]
    Spaces(String),
    #[error("no window has been opened yet — seal something first")]
    NoWindow,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// How much plaintext one published operation carries.
///
/// **A SPACES MESSAGE IS SMALL BY DESIGN, AND THAT IS NOT NEGOTIABLE.** The
/// ciphertext travels inline in `SpacesArgs::Application`, which lives in the
/// operation's header extensions, and `p2panda-core` decodes headers with
/// `.length_limit(512)` and the comment "rather low / pessimistic thresholds".
/// Measured, the practical ceiling is somewhere between 1,376 and 1,548 bytes
/// of records; past it a published operation is written and then fails to
/// decode on the way back out, reported as a corrupted header.
///
/// 1 KB leaves room for the rest of the header, which is not fixed — a space's
/// dependencies are carried in it too. A day of CGM is roughly 23 KB, so a day
/// is about two dozen operations rather than one. That is the shape p2panda
/// expects; it is not a workaround.
const MAX_PAYLOAD: usize = 256;

/// Split a window's records into publishable payloads.
///
/// **BY BYTES, NOT BY RECORD.** Splitting on record boundaries is the obvious
/// design and it cannot work: a single `profile` record in this subject's real
/// history is 566 bytes, which is already over the ceiling, so it could never
/// be published at all no matter how the chunker grouped things.
///
/// Instead a window's payloads are one byte stream. `encode_records` terminates
/// every record with a newline, so a record split across two payloads is
/// rejoined by concatenating them in order and splitting on newlines at the
/// end — which is what [`Vault::ingest`] does, per space.
///
/// The cost is that a reader missing an operation in the middle corrupts the
/// two records either side of the gap rather than just losing one. They fail to
/// parse and are dropped. A missing operation is already data loss; this makes
/// it very slightly worse and much simpler, and `ingest` reports the count of
/// operations it could not process so the loss is never silent.
fn chunk(records: &[Record]) -> Vec<Vec<u8>> {
    encode_records(records)
        .into_bytes()
        .chunks(MAX_PAYLOAD)
        .map(<[u8]>::to_vec)
        .collect()
}

/// How far back a grant reaches./// How far back a grant reaches.
///
/// The choice is offered per grant because the answer is genuinely different
/// for different people: a partner who has been watching for a year wants the
/// year, and someone helping with a single bad night does not need it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Every window, including days sealed before this grant existed.
    Everything,
    /// A fresh window. Nothing sealed before now is readable, ever.
    ///
    /// ⚠️ **NOT YET IMPLEMENTED CORRECTLY HERE.** The version below carries
    /// existing readers into each new window, which puts them in two spaces —
    /// and a reader can belong to exactly one of a subject's spaces
    /// (`spike/p2panda-spaces` §5b: the second join delivers no welcome).
    ///
    /// The arrangement that works, measured in §5c, is the other way round:
    /// every reader stays in the one window they were granted in, and the
    /// subject publishes each day into every live window. Nobody is ever in two
    /// spaces. It costs one copy of each day per live window.
    ///
    /// This also needs the repair discipline from §5a — `spaces_repair_required`
    /// then `repair_spaces_persisted` before **every** auth-level operation —
    /// without which the subject side panics as soon as a second window exists.
    FromNow,
}

pub struct Vault {
    manager: SwarmManager,
    spaces: Store,
    /// Kept so replicated operations can be persisted before being processed;
    /// the manager takes them from the store, not from the caller.
    sqlite: SqliteStore,
    root: PathBuf,
    offset_ms: i64,
    /// How many windows exist. The only state this crate keeps outside the
    /// store, and space ids are derived from it.
    windows: usize,
}

fn windows_path(root: &Path) -> PathBuf {
    root.join("windows")
}

impl Vault {
    /// Open, or create, a vault rooted at a directory.
    ///
    /// **THE IDENTITY IS THE VAULT'S, NOT THE CALLER'S.** A device has to keep
    /// the same credentials for ever: `p2panda-spaces` says neither key can be
    /// rotated without losing access to every space, and a phone that came back
    /// from a reboot as a new member would invalidate every grant anyone had
    /// made to it.
    ///
    /// That is harder than it should be at 0.7.1. `SecretKey::from_bytes` and
    /// `as_bytes` are both `pub(crate)` unless the `test_utils` feature is on,
    /// so the identity secret can be neither exported nor restored as bytes.
    /// The public route is serde: `Credentials` derives `Serialize` and
    /// `Deserialize`, so the whole thing round-trips through a file. Written
    /// 0600, and it is the most sensitive file this project creates — it is
    /// read access to everything the subject has ever published.
    pub async fn open(root: impl AsRef<Path>, offset_ms: i64) -> Result<Self, Error> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(&root)?;
        let credentials = load_or_create_credentials(&root)?;

        let url = format!("sqlite://{}", root.join("spaces.sqlite").display());
        let store = SqliteStoreBuilder::new()
            .database_url(&url)
            .create_database(true)
            .build()
            .await
            .map_err(|e| Error::Store(e.to_string()))?;

        Self::with_store(root, store, credentials, offset_ms).await
    }

    /// The same, on a store the caller owns — an in-memory one, in tests.
    pub async fn with_store(
        root: impl AsRef<Path>,
        store: SqliteStore,
        credentials: Credentials,
        offset_ms: i64,
    ) -> Result<Self, Error> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(&root)?;

        let spaces_store = Store::new(store.clone());
        let spaces_for_vault = spaces_store.clone();
        let forge = SwarmForge::new(store.clone(), credentials.signing_key());

        // REAL RANDOMNESS. `Rng::from_seed` exists but is test-only, which is
        // the right way round: a deterministic RNG in a key layer is a
        // deterministic key layer.
        let rng = Rng::default();

        let manager = SwarmManager::new(spaces_store, forge, credentials, rng)
            .map_err(|e| Error::Spaces(e.to_string()))?;

        let windows = std::fs::read_to_string(windows_path(&root))
            .ok()
            .and_then(|s| s.trim().parse::<usize>().ok())
            .unwrap_or(0);

        Ok(Vault { manager, spaces: spaces_for_vault, sqlite: store, root, offset_ms, windows })
    }

    /// This subject's public key — what a reader is granted against.
    pub fn subject(&self) -> VerifyingKey {
        self.manager.id()
    }

    /// The id of window `n` for this subject.
    ///
    /// Derived rather than random so that a reader holding the subject's key
    /// can name every window without being told, and two devices belonging to
    /// one subject agree without coordinating.
    pub fn window_id(&self, n: usize) -> SpaceId {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"diaswarm/window/1/");
        bytes.extend_from_slice(self.subject().as_bytes());
        bytes.extend_from_slice(&(n as u64).to_be_bytes());
        Hash::digest(&bytes)
    }

    pub fn windows(&self) -> usize {
        self.windows
    }

    fn note_windows(&self) -> Result<(), Error> {
        std::fs::write(windows_path(&self.root), self.windows.to_string())?;
        Ok(())
    }

    async fn space(&self, n: usize) -> Result<SwarmSpace, Error> {
        self.manager
            .space(self.window_id(n))
            .await
            .map_err(|e| Error::Spaces(e.to_string()))?
            .ok_or(Error::NoWindow)
    }

    // ---- persistence glue -------------------------------------------------
    //
    // `p2panda-spaces` has `*_persisted` variants of all of these — and they
    // are `#[cfg(any(test, feature = "test_utils"))]`, so a shipping build
    // cannot call them. The real API hands back the new auth and space state
    // and expects the application to store it. These four helpers are that,
    // and nothing more: they are the test wrappers, rewritten because they are
    // not compiled for us.
    //
    // AND THE STATE SETTERS ARE TEST-ONLY TOO. `Manager::set_groups_state` and
    // `set_space_state` are inside the same `cfg`, so the public API hands back
    // state that the public API cannot store — and `AuthGroupState` is a
    // private alias, so it cannot even be named. The way through is one layer
    // down: `p2panda-store`'s `GroupsStore` and `SpacesStore` are public
    // traits with public methods, and the alias expands to public types. The
    // single thing reproduced from upstream's internals is the key the global
    // auth state is filed under.
    //
    // Worth knowing before trusting a spike: `spike/p2panda-spaces` measured
    // the grant matrix through the test-only wrappers. The property it measured
    // is real, but the integration is larger than it made it look, and this is
    // where the difference lands.

    async fn persist_group(&self, y: &AuthState) -> Result<(), Error> {
        let store = &self.spaces;
        let out: Result<(), p2panda_store::SqliteError> = async {
            tx!(store, {
                store.set_groups_state_tx(Hash::digest(GLOBAL_GROUPS_CONTEXT_ID), y).await?
            });
            Ok(())
        }
        .await;
        out.map_err(|e| Error::Store(e.to_string()))
    }

    async fn persist_space(&self, y: SpacesState<Conditions>) -> Result<(), Error> {
        let state: SpacesStoreState<Conditions> = y.into();
        let store = &self.spaces;
        let out: Result<(), p2panda_store::SqliteError> = async {
            tx!(store, { store.set_space_state_tx(&state.space_id, &state).await? });
            Ok(())
        }
        .await;
        out.map_err(|e| Error::Store(e.to_string()))
    }

    /// Bring every stale space up to date with the shared auth state.
    ///
    /// **BEFORE EVERY AUTH-LEVEL OPERATION, not once before a batch.** All of a
    /// subject's spaces share one global auth state, so creating a space or
    /// changing anyone's membership leaves every *other* space working from an
    /// old view of it — and the next membership change on a stale space panics
    /// `p2panda-auth`. Adding a member is itself an auth change, so two adds in
    /// a row need a repair between them.
    ///
    /// This is what `p2panda-spaces`' own `shared_auth_state` test does on
    /// every line, commented "Make Space 0 aware of this change". Not doing it
    /// is what made a second window look like an upstream defect
    /// (`spike/p2panda-spaces` §5a).
    ///
    /// `repair_spaces_persisted` is test-only like the rest, so the state is
    /// persisted here.
    async fn repair(&self) -> Result<Vec<Operation>, Error> {
        let needs = self
            .manager
            .spaces_repair_required()
            .await
            .map_err(|e| Error::Spaces(e.to_string()))?;
        if needs.is_empty() {
            return Ok(Vec::new());
        }
        let results = self
            .manager
            .repair_spaces(&needs)
            .await
            .map_err(|e| Error::Spaces(e.to_string()))?;
        let mut msgs = Vec::new();
        for (space_y, produced) in results {
            self.persist_space(space_y).await?;
            msgs.extend(produced);
        }
        Ok(msgs)
    }

    /// Open a new window: an empty space, with the stream header in it.
    ///
    /// **EMPTY, AND IT STAYS THAT WAY UNTIL SOMEBODY IS GRANTED INTO IT.** An
    /// earlier version carried every existing reader into each new window,
    /// which is the obvious reading of "nobody loses access because someone
    /// else was granted" and is unimplementable: a reader can belong to exactly
    /// one of a subject's spaces, and the second one it joins hands it no
    /// welcome (§5b). Access is kept alive by [`Vault::seal`] publishing into
    /// every window instead.
    async fn open_window(&mut self) -> Result<Vec<Operation>, Error> {
        let mut msgs = self.repair().await?;
        let n = self.windows;
        let (groups_y, space_y, create) = self
            .manager
            .create_space(self.window_id(n), &[])
            .await
            .map_err(|e| Error::Spaces(e.to_string()))?;
        self.persist_group(&groups_y).await?;
        self.persist_space(space_y).await?;
        self.windows = n + 1;
        self.note_windows()?;
        msgs.extend(create);

        // The stream header goes first in every window, so a reader learns the
        // spec version and the epoch offset from the data itself rather than
        // from anything it had to be told separately.
        msgs.push(self.publish_into(n, encode(&[], self.offset_ms).into_bytes()).await?);
        Ok(msgs)
    }

    /// Everyone who can read window `n`, this device excluded.
    async fn readers_of(&self, n: usize) -> Result<Vec<VerifyingKey>, Error> {
        let space = self.space(n).await?;
        let me = self.subject();
        Ok(space
            .members()
            .await
            .map_err(|e| Error::Spaces(e.to_string()))?
            .into_iter()
            .map(|(id, _)| id)
            .filter(|id| *id != me)
            .collect())
    }

    /// Everyone who can currently read anything, across all windows.
    ///
    /// Read back through `p2panda-spaces`' own API rather than from anything
    /// this crate remembers, which is what makes it a check on persistence
    /// rather than an echo of it.
    pub async fn reader_ids(&self) -> Result<Vec<VerifyingKey>, Error> {
        let mut out: Vec<VerifyingKey> = Vec::new();
        for n in 0..self.windows {
            for id in self.readers_of(n).await? {
                if !out.contains(&id) {
                    out.push(id);
                }
            }
        }
        Ok(out)
    }

    /// Seal a day's records — into **every** live window.
    ///
    /// FAN-OUT, AND IT IS THE WHOLE DESIGN. A reader can belong to exactly one
    /// of a subject's spaces, so windows cannot share readers and a reader
    /// cannot be moved forward into a newer window. What keeps everyone
    /// receiving is that the day is published into all of them: a reader
    /// granted long ago is still in window 0, a reader granted "from now on"
    /// last week is in window 3, and each gets the same day encrypted under
    /// their own window's secret.
    ///
    /// **This costs one copy of the ciphertext per live window**, which is the
    /// price of the per-grant history choice and is stated in the README rather
    /// than hidden here.
    ///
    /// Append-only, unlike the hand-rolled vault, which re-sealed a whole
    /// segment on every call — reading the plaintext back, merging and writing
    /// over it. Writing over it with only the newest records once cost a
    /// subject 4,276 records from the only copy they had. There is nothing to
    /// overwrite here.
    pub async fn seal(&mut self, records: &[Record]) -> Result<Vec<Operation>, Error> {
        let mut msgs = Vec::new();
        if self.windows == 0 {
            msgs.extend(self.open_window().await?);
        }
        if records.is_empty() {
            return Ok(msgs);
        }
        let payloads = chunk(records);
        for n in 0..self.windows {
            for payload in &payloads {
                msgs.push(self.publish_into(n, payload.clone()).await?);
            }
        }
        Ok(msgs)
    }

    async fn publish_into(&self, n: usize, payload: Vec<u8>) -> Result<Operation, Error> {
        let space = self.space(n).await?;
        let (space_y, msg) =
            space.publish(&payload).await.map_err(|e| Error::Spaces(e.to_string()))?;
        self.persist_space(space_y).await?;
        Ok(msg)
    }

    /// Let a reader in, as far back as they are being given.
    pub async fn grant(
        &mut self,
        reader: VerifyingKey,
        reach: Reach,
    ) -> Result<Vec<Operation>, Error> {
        let mut msgs = Vec::new();
        if self.windows == 0 {
            msgs.extend(self.open_window().await?);
        }

        let window = match reach {
            // Window 0 holds everything the subject has ever sealed, and keeps
            // receiving, so joining it is "the lot, from the beginning".
            Reach::Everything => 0,
            // A window that did not exist until now cannot contain anything
            // published before now. There is nothing to withhold.
            Reach::FromNow => {
                msgs.extend(self.open_window().await?);
                self.windows - 1
            }
        };

        msgs.extend(self.repair().await?);
        let space = self.space(window).await?;
        let (groups_y, space_y, auth_msg, space_msg) = space
            .add(reader, Access::read())
            .await
            .map_err(|e| Error::Spaces(e.to_string()))?;
        self.persist_group(&groups_y).await?;
        self.persist_space(space_y).await?;
        msgs.push(auth_msg);
        msgs.push(space_msg);
        Ok(msgs)
    }

    /// Take a reader's access away, from now on.
    ///
    /// Prospective only, and nothing here pretends otherwise: what they have
    /// already downloaded stays readable for ever. What changes is that the
    /// next thing sealed is encrypted under a secret they do not have.
    pub async fn revoke(&mut self, reader: VerifyingKey) -> Result<Vec<Operation>, Error> {
        let mut msgs = Vec::new();
        for n in 0..self.windows {
            if !self.readers_of(n).await?.contains(&reader) {
                continue;
            }
            msgs.extend(self.repair().await?);
            let space = self.space(n).await?;
            let (groups_y, space_y, auth_msg, space_msg) = space
                .remove(reader)
                .await
                .map_err(|e| Error::Spaces(e.to_string()))?;
            self.persist_group(&groups_y).await?;
            self.persist_space(space_y).await?;
            msgs.push(auth_msg);
            msgs.push(space_msg);
        }
        Ok(msgs)
    }

    /// Tell this vault about another peer, so it can be granted.
    ///
    /// On a phone this is what scanning an invite does.
    pub async fn register(&self, other: &Vault) -> Result<(), Error> {
        let me = other.manager.me().await.map_err(|e| Error::Spaces(e.to_string()))?;
        self.manager.register_member(&me).await.map_err(|e| Error::Spaces(e.to_string()))
    }

    /// Process operations replicated from somewhere else, and return whatever
    /// they turned out to contain that this vault can read.
    ///
    /// **EVERY OPERATION IS STORED WHETHER OR NOT IT OPENS.** That is the
    /// property the swarm rests on: a peer holds ciphertext for people it has
    /// never met. An operation that yields nothing is not an error, it is the
    /// access control working.
    ///
    /// PANICS ARE CAUGHT, deliberately. `p2panda-auth` panics rather than
    /// erroring when an operation arrives without its dependencies — measured
    /// at `group/crdt/mod.rs:727` and `group/resolver.rs:276`, and sometimes
    /// the same situation returns a clean error instead. Log sync delivers in
    /// dependency order and should make it unreachable, but this runs in a
    /// background worker on a phone driving an insulin pump, and a panic there
    /// takes the app down. An unreadable day is a bad afternoon; an app that
    /// will not start is a loop that has stopped.
    pub async fn ingest(&self, ops: &[Operation]) -> Result<Ingested, Error> {
        // Per space, in arrival order: see [`chunk`]. Records span payloads,
        // so a window's payloads have to be rejoined before anything is parsed,
        // and rejoining two different windows' streams would splice records
        // that were never adjacent.
        let mut streams: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut refused = 0usize;
        let mut panicked = 0usize;
        for op in ops {
            if self.store_operation(op).await.is_err() {
                refused += 1;
                continue;
            }
            // A READER NEEDS THE REPAIR DISCIPLINE TOO. Processing a space's
            // messages gives this peer state for that space whether or not it
            // is a member, so a reader that has seen two of a subject's windows
            // has two spaces to keep current — and the same stale-auth-state
            // panic as the subject.
            //
            // Only before auth-carrying operations. Repair queries the store,
            // and a year of records is tens of thousands of application
            // messages; doing it before each one made ingest quadratic in the
            // history for no benefit, since an application message cannot
            // change anybody's membership.
            if !matches!(Borrow::<SpacesArgs<Conditions>>::borrow(op), SpacesArgs::Application { .. })
            {
                let _ = self.repair().await;
            }

            let processed = std::panic::AssertUnwindSafe(self.manager.process(op))
                .catch_unwind()
                .await;
            let (groups_y, space_y, events) = match processed {
                Ok(Ok(out)) => out,
                Ok(Err(_)) => {
                    refused += 1;
                    continue;
                }
                Err(_) => {
                    panicked += 1;
                    continue;
                }
            };
            if let Some(y) = groups_y {
                let _ = self.persist_group(&y).await;
            }
            if let Some(y) = space_y {
                let _ = self.persist_space(y).await;
            }
            for e in events {
                if let Event::Application { space_id, data } = e {
                    streams.entry(space_id.to_hex()).or_default().extend_from_slice(&data);
                }
            }
        }
        Ok(Ingested { records: decode_records(&streams), refused, panicked })
    }

    async fn store_operation(&self, op: &Operation) -> Result<(), Error> {
        use p2panda_store::operations::OperationStore;
        let store = &self.sqlite;
        let out: Result<bool, p2panda_store::SqliteError> = async {
            Ok(tx!(store, { store.insert_operation(&op.hash(), op.inner(), &LOG_ID).await? }))
        }
        .await;
        out.map(|_| ()).map_err(|e| Error::Store(e.to_string()))
    }

}

/// What one pass of [`Vault::ingest`] did.
///
/// **`refused` AND `panicked` ARE PART OF THE ANSWER, NOT DIAGNOSTICS.** An
/// operation this vault cannot process is usually access control working
/// exactly as intended — a stranger's space, a day sealed after a revocation.
/// It is occasionally a dependency that has not arrived, and then the records
/// that came back are an incomplete history that looks like a complete one.
/// Returning the counts is what lets a caller tell the two apart; the earlier
/// version of this dropped both on the floor, and a reader that silently
/// received four days out of five is precisely the failure this project keeps
/// being bitten by.
#[derive(Debug, Clone, Default)]
pub struct Ingested {
    pub records: Vec<Record>,
    /// Operations the library declined, with an error.
    pub refused: usize,
    /// Operations that panicked p2panda-auth. See [`Vault::ingest`].
    pub panicked: usize,
}

/// Parse each window's rejoined byte stream back into records.
///
/// Lines that do not parse are dropped rather than failing the read: a reader
/// that can open four of a subject's five windows should see four windows of
/// data, not an error. The same tolerance covers the two records either side of
/// a missing operation.
fn decode_records(streams: &BTreeMap<String, Vec<u8>>) -> Vec<Record> {
    let mut out = Vec::new();
    for bytes in streams.values() {
        for line in String::from_utf8_lossy(bytes).lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(r) = Record::from_json(line) {
                out.push(r);
            }
        }
    }
    out
}

fn credentials_path(root: &Path) -> PathBuf {
    root.join("credentials.json")
}

/// Load this device's credentials, creating them the first time.
///
/// Never served, never replicated, and 0600 on any platform that has modes —
/// the same treatment `diaswarm-core` gives `readers.json`, for a stronger
/// reason: this file is the identity itself.
fn load_or_create_credentials(root: &Path) -> Result<Credentials, Error> {
    let path = credentials_path(root);
    if let Ok(bytes) = std::fs::read(&path) {
        if let Ok(c) = serde_json::from_slice::<Credentials>(&bytes) {
            return Ok(c);
        }
        // REFUSE RATHER THAN REGENERATE. A corrupt credentials file that is
        // silently replaced looks like it worked and quietly abandons every
        // grant the subject ever made, with the old identity still named in
        // other people's spaces.
        return Err(Error::Spaces(format!(
            "{} exists but could not be read — refusing to replace it, because a new \
             identity would abandon every grant made to this device",
            path.display()
        )));
    }

    let credentials =
        Credentials::from_rng(&Rng::default()).map_err(|e| Error::Spaces(e.to_string()))?;
    let json = serde_json::to_vec(&credentials).map_err(|e| Error::Spaces(e.to_string()))?;
    std::fs::write(&path, json)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(credentials)
}
