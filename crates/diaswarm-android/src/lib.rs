//! The JNI surface both apps call.
//!
//! **ONE CONTRACT, TWO CALLERS.** The symbol names are derived from the Kotlin
//! package and class, so they can belong to only one package — and there are now
//! two apps: the AAPS add-on that publishes a loop's records, and the standalone
//! follower that reads somebody else's. Neither owns this, so the package is
//! `nz.diaswarm.jni` in both and neither app's namespace leaks into the other.
//! Renaming either half breaks the other AT LOAD TIME ON A PHONE, not at compile
//! time on a desktop, so the two move together or not at all.
//!
//! Deliberately thin. Everything with a decision in it lives in `diaswarm-core`,
//! which is asserted byte-identical to `tools/canon.py` over the whole reference
//! snapshot; this file only moves strings across the boundary. If logic starts
//! accumulating here it has escaped the one place the spec is implemented, which
//! is the entire reason the record code went over the NDK instead of being
//! written again in Kotlin.
//!
//! The Kotlin side owns what this cannot: reading `DataSyncSelector`'s typed
//! `DataPair`s, checking `isValid` (the sync queue does not, and xdrip never
//! does either), and keeping its own high-water marks.

use jni::objects::{JClass, JString};
use jni::sys::{jint, jlong, jboolean};
use jni::JNIEnv;

use std::path::{Path, PathBuf};

use diaswarm_core::vault::{hex, Identity, Vault};
use diaswarm_core::{epoch_of, header, Emitted, Record};

/// Give the network stack Android's `Context`, once, before anything else.
///
/// **WITHOUT THIS, A TASK DIES EVERY RUN AND NOTHING SAYS SO.** iroh watches for
/// network changes by asking Android for a ConnectivityManager through
/// `ndk_context`, which panics with "android context was not initialized" if
/// nobody has handed one over. That panic happens inside a tokio task, so tokio
/// catches it, the task disappears, and the swarm carries on — without ever
/// noticing that wifi dropped and mobile data took over. It was doing exactly
/// that on a phone driving a pump for as long as this has existed, and it was
/// only found because turning on `panic = "abort"` for a smaller binary turned
/// the silence into a crash on launch.
///
/// The Context is held as a global ref for the life of the process on purpose:
/// `ndk_context` keeps the raw pointer, so letting the JNI local reference lapse
/// would leave it dangling.
///
/// Safe to call more than once; the second call does nothing rather than
/// re-registering a second Context.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_initAndroid<'a>(
    env: JNIEnv<'a>,
    _class: JClass<'a>,
    context: jni::objects::JObject<'a>,
) {
    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if DONE.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let Ok(vm) = env.get_java_vm() else { return };
    let Ok(global) = env.new_global_ref(context) else { return };
    unsafe {
        ndk_context::initialize_android_context(
            vm.get_java_vm_pointer() as *mut std::ffi::c_void,
            global.as_raw() as *mut std::ffi::c_void,
        );
    }
    // The pointer above outlives this scope, so the reference has to as well.
    std::mem::forget(global);
}

/// Which version of `spec/records.md` this build implements.
///
/// The plugin should refuse to publish if this disagrees with what it expects:
/// a silently mismatched native library is how a stream ends up conforming to a
/// spec nobody thinks it conforms to.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_specVersion(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    diaswarm_core::SPEC_VERSION as jint
}

/// Which epoch a timestamp falls in — the unit of key custody, a UTC day.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_epochOf(
    _env: JNIEnv,
    _class: JClass,
    t: jlong,
    offset_ms: jlong,
) -> jlong {
    epoch_of(t, offset_ms)
}

/// The stream header (spec §5.2), as a canonical JSON line.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_header<'a>(
    env: JNIEnv<'a>,
    _class: JClass<'a>,
    offset_ms: jlong,
) -> JString<'a> {
    to_jstring(env, header(offset_ms).to_canonical_json())
}

/// Canonicalise one record: sorted keys, absent fields omitted, no spaces.
///
/// Returns an empty string if the input is not a record, rather than throwing —
/// a malformed record on a loop phone must not become an exception on a
/// background thread in a medical device's process.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_canonicalLine<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    input: JString<'a>,
) -> JString<'a> {
    let Ok(raw) = env.get_string(&input) else {
        return to_jstring(env, String::new());
    };
    let raw: String = raw.into();
    match Record::from_json(&raw) {
        Ok(record) => to_jstring(env, record.normalise().to_canonical_json()),
        Err(_) => to_jstring(env, String::new()),
    }
}

/// A deduplicating emitter, held across calls.
///
/// The AAPS sync queue re-emits the current record on every edit (see
/// `Emitted`), so the plugin needs state that survives between records. Kotlin
/// holds the pointer and must call `emitterFree` — there is no finaliser, on
/// purpose, because relying on one to release native state in an app that must
/// keep dosing is worse than leaking.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_emitterNew(
    _env: JNIEnv,
    _class: JClass,
    last_cgm_bucket: jlong,
) -> jlong {
    // NEGATIVE MEANS "NOTHING EMITTED YET". A bucket is a count of five-minute
    // periods since the epoch and is never negative, so there is no value to
    // confuse it with — and the caller stores it in a `Long` preference whose
    // default is 0, which would otherwise mean "bucket zero, 1970" and thin
    // every reading the phone has.
    let mark = if last_cgm_bucket < 0 { None } else { Some(last_cgm_bucket as i64) };
    Box::into_raw(Box::new(Emitted::resuming(mark))) as jlong
}

/// The newest CGM bucket emitted, for the caller to persist. -1 if none.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_emitterLastCgmBucket(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jlong {
    if handle == 0 {
        return -1;
    }
    let emitter = unsafe { &*(handle as *const Emitted) };
    emitter.last_cgm_bucket().unwrap_or(-1)
}

/// Always 0. Nothing is thinned any more — see `Emitted::accept` and the
/// withdrawal of spec §3.3.
///
/// Kept so the Kotlin side keeps linking while the preference and the call that
/// carried the thinning mark are retired deliberately, rather than in the same
/// change that stopped losing readings.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_emitterThinned(
    _env: JNIEnv,
    _class: JClass,
    _handle: jlong,
) -> jlong {
    0
}

#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_emitterFree(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    if handle != 0 {
        unsafe { drop(Box::from_raw(handle as *mut Emitted)) };
    }
}

/// Offer a record to the emitter. Returns the canonical line to publish, or an
/// empty string if it has already been emitted.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_emitterAccept<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
    input: JString<'a>,
) -> JString<'a> {
    if handle == 0 {
        return to_jstring(env, String::new());
    }
    let emitter = unsafe { &mut *(handle as *mut Emitted) };
    let Ok(raw) = env.get_string(&input) else {
        return to_jstring(env, String::new());
    };
    let raw: String = raw.into();
    let Ok(record) = Record::from_json(&raw) else {
        return to_jstring(env, String::new());
    };
    // Normalise BEFORE the duplicate check: two raw records differing only below
    // the precision the device has are the same record, and must not both be
    // emitted just because Kotlin handed over unrounded doubles.
    let record = record.normalise();
    if emitter.accept(&record) {
        to_jstring(env, record.to_canonical_json())
    } else {
        to_jstring(env, String::new())
    }
}

/// How many genuine edits this emitter has seen — records that differed from
/// something already emitted at the same instant and kind.
///
/// Counted rather than handled. spec §7 reserves an `amend` kind and says to
/// measure before designing one, because the reference snapshot showed roughly
/// fifteen a year and that is too few to guess a mechanism from.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_emitterAmendments(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jlong {
    if handle == 0 {
        return 0;
    }
    let emitter = unsafe { &*(handle as *const Emitted) };
    emitter.amendments as jlong
}

// ---------------------------------------------------------------------------
// The vault
// ---------------------------------------------------------------------------
//
// STATELESS ON PURPOSE. The emitter above is a raw pointer because it has to
// live across a whole upload; these do not. Paths in, result out, nothing to
// leak and nothing to free — in an app that must keep dosing, a handle whose
// lifecycle spans a background worker is a liability that buys nothing.
//
// NOTHING HERE THROWS. A malformed argument or an unwritable disk returns a
// negative number or an empty string. An exception crossing JNI on a
// background thread inside a medical device's process is not a diagnostic, it
// is a crash.

/// Load the subject identity, creating it on first use.
///
/// The file holds secret keys and is the whole of what being this subject
/// means: lose it and every future grant is lost with it, because the epoch
/// keys it protects cannot be re-derived.
fn load_or_create_identity(path: &Path) -> Option<Identity> {
    if let Ok(raw) = std::fs::read(path) {
        let bytes: [u8; 64] = raw.try_into().ok()?;
        return Some(Identity::from_bytes(&bytes));
    }
    let id = Identity::generate();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    std::fs::write(path, id.to_bytes()).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Some(id)
}

fn open_or_create_vault(vault: &Path, subject: &Identity, offset: i64) -> Option<Vault> {
    if vault.join("meta.json").exists() {
        Vault::open(vault).ok()
    } else {
        Vault::create(vault, subject, offset).ok()
    }
}

/// Seal one epoch of canonical NDJSON into the vault.
///
/// Returns the number of records sealed, or a negative number: -1 bad
/// arguments, -2 identity unavailable, -3 vault unavailable, -4 seal failed.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_vaultSeal<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    vault_path: JString<'a>,
    identity_path: JString<'a>,
    epoch: jlong,
    offset_ms: jlong,
    ndjson: JString<'a>,
) -> jlong {
    let (Ok(vault_s), Ok(id_s), Ok(body)) = (
        env.get_string(&vault_path),
        env.get_string(&identity_path),
        env.get_string(&ndjson),
    ) else {
        return -1;
    };
    let vault_p = PathBuf::from(String::from(vault_s));
    let id_p = PathBuf::from(String::from(id_s));
    let body = String::from(body);

    let Some(subject) = load_or_create_identity(&id_p) else { return -2 };
    let Some(vault) = open_or_create_vault(&vault_p, &subject, offset_ms) else { return -3 };

    // Publish the key the grant log is signed with, if an older build did not.
    // Sealing is the operation that runs constantly and has the identity, so a
    // vault heals here without anyone having to be told to migrate it.
    let _ = vault.ensure_signer(&subject);

    let records: Vec<Record> = body
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| Record::from_json(l).ok())
        .map(Record::normalise)
        .collect();

    match vault.seal(epoch, &records) {
        Ok(_) => records.len() as jlong,
        Err(_) => -4,
    }
}

/// The subject's public key, for handing to someone. Empty on failure.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_vaultSubject<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    identity_path: JString<'a>,
) -> JString<'a> {
    let Ok(id_s) = env.get_string(&identity_path) else {
        return to_jstring(env, String::new());
    };
    match load_or_create_identity(&PathBuf::from(String::from(id_s))) {
        Some(id) => to_jstring(env, hex(&id.enc_public())),
        None => to_jstring(env, String::new()),
    }
}

/// A one-line summary of the vault, for a log line or a status row.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_vaultStatus<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    vault_path: JString<'a>,
) -> JString<'a> {
    let Ok(vault_s) = env.get_string(&vault_path) else {
        return to_jstring(env, String::new());
    };
    let p = PathBuf::from(String::from(vault_s));
    let Ok(vault) = Vault::open(&p) else {
        return to_jstring(env, String::from("no vault"));
    };
    let epochs = vault.epochs().unwrap_or_default();
    let grants = vault.grants().map(|g| g.len()).unwrap_or(0);
    let summary = match (epochs.first(), epochs.last()) {
        (Some(a), Some(b)) => format!("{} epochs {a}..{b}, {grants} grants", epochs.len()),
        _ => format!("empty, {grants} grants"),
    };
    to_jstring(env, summary)
}

// ---------------------------------------------------------------------------
// The pool
// ---------------------------------------------------------------------------
//
// Turning swarm on puts this phone in a pool: it finds peers, works out which
// slice of the subject space is its share, and holds what falls there — for
// people it has never met and cannot read.

/// A running pool membership: the runtime and the swarm it owns.
struct Pooled {
    runtime: tokio::runtime::Runtime,
    swarm: diaswarm_net::swarm::Swarm,
    node_id: String,
    /// **THE ONE KEYS STORE ON THIS PHONE, AND IT LIVES HERE FOR A REASON.**
    ///
    /// `p2panda-store` builds its pool with `max_connections(1)` and sets no
    /// busy timeout. Two independent pools on one SQLite file is therefore not
    /// a tidiness question but a writer-contention bug: sealing and replication
    /// would take turns failing, on a phone that may be driving an insulin
    /// pump. So there is exactly one, shared by cloning the handle — and it
    /// belongs to the pool because the pool is the long-lived object. Every
    /// other keys handle is opened and closed within a pass.
    ///
    /// `None` when the phone joined without a keys directory, which is what a
    /// build with the keys vault switched off looks like.
    keys_store: Option<diaswarm_keys::SqliteStore>,
    /// Replication for that store. Holding it here keeps its subscription
    /// alive: dropping a `KeysReplicator` unsubscribes, and a follower that
    /// re-subscribed every pass would be catching up for ever.
    keys_replicator: Option<diaswarm_net::replicate::KeysReplicator>,
}

/// Join the pool. Returns a handle, or 0.
///
/// Uses the SAME node key file as before, so the phone keeps the endpoint id it
/// already had. Everything that ranks peers ranks them by that id, and a new
/// key would read as one peer leaving and another arriving — every bucket it
/// held would reshuffle for nothing.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_swarmJoin<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    store_path: JString<'a>,
    node_key_path: JString<'a>,
    keys_path: JString<'a>,
) -> jlong {
    // Empty means this build has no keys vault: it pools, serves and follows
    // exactly as before, which is what keeps D26 switch-off-able.
    let keys_dir = env
        .get_string(&keys_path)
        .ok()
        .map(String::from)
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from);
    let (Ok(store), Ok(key_path)) = (env.get_string(&store_path), env.get_string(&node_key_path))
    else {
        return 0;
    };
    let store = PathBuf::from(String::from(store));
    let key_path = PathBuf::from(String::from(key_path));

    let Some(raw) = load_or_create_node_key(&key_path) else { return 0 };
    let signing = p2panda_core::SigningKey::from_bytes(&raw);

    let Ok(runtime) = tokio::runtime::Builder::new_multi_thread().enable_all().build() else {
        return 0;
    };
    let joined = runtime.block_on(diaswarm_net::swarm::Swarm::join(store, signing));
    let Ok(swarm) = joined else { return 0 };
    let Ok(node_id) = runtime.block_on(swarm.node_id()) else { return 0 };

    // The keys store and its replication, on this same runtime. Failure here is
    // not fatal: a phone with no keys vault still pools, serves and follows
    // exactly as it did before, which is what keeps this switch-off-able.
    let (keys_store, keys_replicator) = match keys_dir {
        Some(dir) => {
            // **CREATE THE DIRECTORY FIRST, AND THIS IS WHY THE LOOP PHONE
            // COULD NOT OPEN A KEYS VAULT AT ALL.** SQLite will create the
            // database file but not the directory holding it, so on any device
            // where `files/diaswarm/keys/` did not already exist the store
            // build failed, there was no replicator, and every pass reported
            // `keys carry unavailable (-3)` and `shadow vault would not open`.
            //
            // The test phone hid it: an earlier build's `keysOpen` made the
            // directory itself, so by the time this code ran it was there. A
            // device that had never run that build — which is every device but
            // one — failed every time.
            if let Err(e) = std::fs::create_dir_all(&dir) {
                eprintln!("diaswarm: keys directory {}: {e}", dir.display());
            }
            let url = format!("sqlite://{}", dir.join("keys.sqlite").display());
            match runtime.block_on(async {
                let store = diaswarm_keys::SqliteStoreBuilder::new()
                    .database_url(&url)
                    .create_database(true)
                    .build()
                    .await
                    .map_err(|e| e.to_string())?;
                let (endpoint, gossip) = swarm.parts();
                let replicator = diaswarm_net::replicate::KeysReplicator::keys(
                    store.clone(),
                    endpoint,
                    gossip,
                )
                .await
                .map_err(|e| e.to_string())?;
                Ok::<_, String>((store, replicator))
            }) {
                Ok((store, repl)) => (Some(store), Some(repl)),
                Err(e) => {
                    // **SAY WHY.** Swallowing this is what made the failure
                    // above undiagnosable from the device: the phone could
                    // report that it had no keys store and not one word about
                    // the reason.
                    eprintln!("diaswarm: keys store unavailable: {e}");
                    (None, None)
                }
            }
        }
        None => (None, None),
    };

    Box::into_raw(Box::new(Pooled { runtime, swarm, node_id, keys_store, keys_replicator }))
        as jlong
}

/// Carry a subject's keys logs on the bucket topic they fall in.
///
/// Idempotent: a pool pass calls it for everything it should hold and most of
/// that is already known. Returns 0, or negative.
///
/// **BOTH LOGS, ONE CALL.** `KeysReplicator` associates the segment log and the
/// control log together, because a follower with segments and no grants cannot
/// open them and one with grants and no segments has nothing to open.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_keysCarry<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
    subject_hex: JString<'a>,
) -> jlong {
    if handle == 0 {
        return -1;
    }
    let Ok(subject) = env.get_string(&subject_hex) else { return -2 };
    let subject = String::from(subject).to_ascii_lowercase();
    if verifying_key(&subject).is_none() {
        return -2;
    }
    let pooled = unsafe { &*(handle as *const Pooled) };
    let Some(replicator) = pooled.keys_replicator.as_ref() else { return -3 };

    let members = pooled
        .runtime
        .block_on(pooled.swarm.pool_members())
        .map(|m| m.len())
        .unwrap_or(2)
        .max(2);
    let depth = diaswarm_net::pool::depth_for(members);
    let topic = diaswarm_net::pool::bucket_topic(
        depth,
        diaswarm_net::pool::bucket_of(&subject, depth),
    );
    match pooled.runtime.block_on(replicator.carry(topic, &subject)) {
        Ok(()) => 0,
        Err(_) => -4,
    }
}


/// This phone's id in the pool. Empty on a bad handle.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_swarmNodeId<'a>(
    env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
) -> JString<'a> {
    if handle == 0 {
        return to_jstring(env, String::new());
    }
    let pooled = unsafe { &*(handle as *const Pooled) };
    to_jstring(env, pooled.node_id.clone())
}

/// Accept a pushed invite for the next `seconds`, because the person holding
/// this phone just put their code on screen for somebody to scan.
///
/// See `Request::Offer`: this is the window that stops a stranger who knows
/// this node id from making the phone carry their ciphertext.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_swarmExpectOffer(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    seconds: jlong,
) {
    if handle == 0 {
        return;
    }
    let pooled = unsafe { &*(handle as *const Pooled) };
    pooled.swarm.expect_offer(seconds);
}

/// Hand our invite to somebody whose invite we just scanned, so they do not
/// have to scan one back. 1 taken, 0 declined, <0 could not be delivered.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_swarmOffer<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
    their_endpoint: JString<'a>,
    their_relay: JString<'a>,
    our_invite: JString<'a>,
) -> jlong {
    if handle == 0 {
        return -1;
    }
    let (Ok(e), Ok(r), Ok(o)) = (
        env.get_string(&their_endpoint),
        env.get_string(&their_relay),
        env.get_string(&our_invite),
    ) else {
        return -2;
    };
    let pooled = unsafe { &*(handle as *const Pooled) };
    match pooled.runtime.block_on(pooled.swarm.offer_to(
        &String::from(e),
        &String::from(r),
        &String::from(o),
    )) {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(_) => -3,
    }
}

/// One pass: say what we hold, hear what we should, take on a few of them.
///
/// Returns `pool<TAB>buckets<TAB>held<TAB>wanted<TAB>adopted`, or empty.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_swarmTick<'a>(
    env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
    max_adopt: jlong,
) -> JString<'a> {
    if handle == 0 {
        return to_jstring(env, String::new());
    }
    let pooled = unsafe { &*(handle as *const Pooled) };
    let take = max_adopt.max(0) as usize;
    match pooled.runtime.block_on(pooled.swarm.tick_and_adopt(take)) {
        Ok((r, adopted)) => to_jstring(
            env,
            format!("{}\t{}\t{}\t{}\t{}", r.pool, r.buckets, r.held, r.wanted.len(), adopted),
        ),
        Err(_) => to_jstring(env, String::new()),
    }
}

/// Leave the pool and release the handle. Idempotent on 0.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_swarmLeave<'a>(
    _env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
) {
    if handle == 0 {
        return;
    }
    // Dropping the runtime stops the actors p2panda spawned; dropping the swarm
    // stops discovery and gossip. Order matters only in that both must happen.
    let pooled = unsafe { Box::from_raw(handle as *mut Pooled) };
    drop(pooled);
}

/// Read a 32-byte node key, creating one if absent.
fn load_or_create_node_key(path: &Path) -> Option<[u8; 32]> {
    if let Ok(raw) = std::fs::read(path) {
        if raw.len() == 32 {
            let mut k = [0u8; 32];
            k.copy_from_slice(&raw);
            return Some(k);
        }
    }
    let k: [u8; 32] = iroh::SecretKey::generate().to_bytes();
    std::fs::write(path, k).ok()?;
    Some(k)
}

// ---------------------------------------------------------------------------
// Following, from the phone
// ---------------------------------------------------------------------------
//
// The phone was a publisher: it sealed its own history and served it, and had
// no way to hold anyone else's. That is only half a peer, and it is the half
// that cannot be trialled — two phones where neither can follow the other have
// nothing to show each other.

/// Start keeping a copy of whoever sent this invite. Returns 1 if anything
/// changed, 0 if it was already known, negative on failure.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_netFollow<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    store_path: JString<'a>,
    invite_text: JString<'a>,
) -> jlong {
    let (Ok(store), Ok(text)) = (env.get_string(&store_path), env.get_string(&invite_text)) else {
        return -1;
    };
    let Ok(inv) = diaswarm_core::invite::Invite::parse(&String::from(text)) else { return -2 };
    match diaswarm_net::peer::add_follow_via(
        Path::new(&String::from(store)),
        &inv.subject,
        &inv.endpoint,
        Some(&inv.purpose),
        // Their relay, as they published it — see peer::Follow::relay.
        Some(inv.relay.as_str()),
        // Kept for when this reader joins the subject's keys group — which
        // happens when the welcome replicates, not when the invite is scanned.
        (!inv.keys.is_empty()).then_some(inv.keys.as_str()),
    ) {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(_) => -3,
    }
}

/// Bring every followed subject up to date. Returns how many were reached, or
/// a negative code. Blocking: the caller is already a worker thread.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_netRefresh<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
    store_path: JString<'a>,
) -> jlong {
    let Ok(store) = env.get_string(&store_path) else { return -1 };
    let store = PathBuf::from(String::from(store));

    // REFRESH THROUGH THE POOL, when this phone is in one.
    //
    // Not merely "use the endpoint we serve on", though it does that too. A
    // follower scanned one address, so on its own it is exactly as available as
    // the subject's phone. Going through the swarm means that when that phone
    // is asleep, a peer that announced holding the subject is asked instead —
    // and it is asked by node id, so no address is written down anywhere.
    if handle != 0 {
        let pooled = unsafe { &*(handle as *const Pooled) };
        return match pooled.runtime.block_on(pooled.swarm.refresh_follows()) {
            Ok(results) => results.iter().filter(|r| r.reached()).count() as jlong,
            Err(_) => -3,
        };
    }

    // Not in the pool — the subject's own address is the only one there is.
    let Ok(runtime) = tokio::runtime::Runtime::new() else { return -2 };
    match runtime.block_on(diaswarm_net::peer::refresh_all(&store, false)) {
        Ok(results) => results.iter().filter(|r| r.reached()).count() as jlong,
        Err(_) => -3,
    }
}

/// What this phone follows, one per line: `subject<TAB>purpose<TAB>reached`.
///
/// `reached` is not stored — it is whether anything has ever arrived, judged by
/// the replica existing on disk. A follower that has never reached anyone and
/// one that is merely quiet must not look alike (§12.3).
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_netFollowing<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    store_path: JString<'a>,
) -> JString<'a> {
    let Ok(store) = env.get_string(&store_path) else { return to_jstring(env, String::new()) };
    let store = PathBuf::from(String::from(store));
    let listing = diaswarm_net::peer::load_follows(&store)
        .unwrap_or_default()
        .iter()
        .map(|f| {
            let held = store.join(&f.subject).join("meta.json").exists();
            format!(
                "{}\t{}\t{}\t{}",
                f.subject,
                f.purpose.clone().unwrap_or_else(|| "relay".into()),
                if held { "1" } else { "0" },
                // **A FOURTH COLUMN, EMPTY FOR ANYBODY PAIRED BEFORE D26.** A
                // follower needs this to read the keys vault at all: it holds
                // the log author and the bundle it was granted against. Empty
                // means the core vault and nothing else, which is every subject
                // paired before the field existed.
                f.keys.clone().unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    to_jstring(env, listing)
}

/// The most recent glucose reading this phone can open for a subject it
/// follows, as `mgdl<TAB>millis`, or empty if it can open nothing.
///
/// THE AGE IS RETURNED, NOT A FRESHNESS VERDICT. A follower's dangerous failure
/// is a number that looks current and is nine hours old, so the caller is
/// handed the timestamp and made to say how old it is.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_netLatest<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    store_path: JString<'a>,
    subject: JString<'a>,
    identity_path: JString<'a>,
    purpose: JString<'a>,
) -> JString<'a> {
    let (Ok(store), Ok(subj), Ok(id), Ok(p)) = (
        env.get_string(&store_path),
        env.get_string(&subject),
        env.get_string(&identity_path),
        env.get_string(&purpose),
    ) else {
        return to_jstring(env, String::new());
    };
    let dir = PathBuf::from(String::from(store)).join(String::from(subj));
    let Some(reader) = load_or_create_identity(Path::new(&String::from(id))) else {
        return to_jstring(env, String::new());
    };
    let Ok(vault) = Vault::open(&dir) else { return to_jstring(env, String::new()) };
    let Ok(opened) = vault.read_as(&reader, &String::from(p)) else {
        return to_jstring(env, String::new());
    };
    let latest = opened
        .values()
        .flatten()
        .filter(|r| r.kind() == "cgm")
        .filter_map(|r| Some((r.t(), r.get("mgdl")?.as_f64()?)))
        .max_by_key(|(t, _)| *t);
    match latest {
        Some((t, mgdl)) => to_jstring(env, format!("{mgdl}\t{t}")),
        None => to_jstring(env, String::new()),
    }
}

/// Every glucose reading this phone can open for a followed subject after
/// `since_ms`, as `millis<TAB>mgdl<TAB>trend<TAB>src` lines, oldest first.
///
/// THE PLURAL OF `netLatest`, AND THAT IS THE WHOLE POINT. A number on a screen
/// says somebody is alive; a line on a graph says what their night was like,
/// and the graph is the thing this project is trying to be better than
/// Nightscout at. The reader has already opened the vault to answer
/// `netLatest`, so this costs the same decryption and returns what it was
/// throwing away.
///
/// `since_ms` is EXCLUSIVE, so a caller holding a high-water mark can pass it
/// straight back without re-reading the reading it already has. `limit` bounds
/// the string that crosses JNI — a Libre 3 produces about 1,586 readings a day
/// and a follower catching up after a week must not try to hand 11,000 of them
/// over in one Java String. A caller that gets exactly `limit` lines should ask
/// again from the last timestamp it saw.
///
/// `trend` and `src` are the Kotlin enum NAMES, not their display text, because
/// that is what `SwarmRecords` put in: `TrendArrow.name` and `SourceSensor.name`.
/// Missing fields come back empty rather than guessed at — a reading whose
/// trend the sensor never reported is not FLAT.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_netGlucose<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    store_path: JString<'a>,
    subject: JString<'a>,
    identity_path: JString<'a>,
    purpose: JString<'a>,
    since_ms: jlong,
    limit: jint,
) -> JString<'a> {
    let (Ok(store), Ok(subj), Ok(id), Ok(p)) = (
        env.get_string(&store_path),
        env.get_string(&subject),
        env.get_string(&identity_path),
        env.get_string(&purpose),
    ) else {
        return to_jstring(env, String::new());
    };
    let dir = PathBuf::from(String::from(store)).join(String::from(subj));
    let Some(reader) = load_or_create_identity(Path::new(&String::from(id))) else {
        return to_jstring(env, String::new());
    };
    let Ok(vault) = Vault::open(&dir) else { return to_jstring(env, String::new()) };

    // OPEN THE RECENT END, NOT THE WHOLE GRANT. `read_as` decrypts every
    // segment the reader holds a wrap for and keeps the canonical form of every
    // record in a set to deduplicate overlapping segments — for this subject's
    // own 74 days that is around 150,000 records and several megabytes, and
    // this runs on a two-minute poll rather than on a button. The epoch is on
    // the segment filename, so anything that cannot hold a reading newer than
    // `since_ms` is skipped before it is read.
    //
    // A whole epoch of slack on purpose: `since_ms` falls inside an epoch and
    // deduplication is per call, so the epoch containing it must be opened
    // whole. `since_ms <= 0` means "no mark yet" and reads everything.
    let from_epoch = if since_ms > 0 {
        epoch_of(since_ms, vault.offset())
    } else {
        i64::MIN
    };
    let Ok(opened) = vault.read_as_from(&reader, &String::from(p), from_epoch) else {
        return to_jstring(env, String::new());
    };

    // Sorted and deduplicated by timestamp: segments legitimately overlap (a
    // rotation restarts one for the same epoch), so the same reading can be in
    // the map twice. The consumer inserts these into a database keyed on
    // (timestamp, sensor) and would merely do redundant work, but a follower
    // counting what it received should be told the truth.
    let mut rows: Vec<(i64, f64, String, String)> = opened
        .values()
        .flatten()
        .filter(|r| r.kind() == "cgm")
        .filter(|r| r.t() > since_ms)
        .filter_map(|r| {
            Some((
                r.t(),
                r.get("mgdl")?.as_f64()?,
                r.get("trend").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                r.get("src").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            ))
        })
        .collect();
    rows.sort_by_key(|(t, _, _, _)| *t);
    rows.dedup_by_key(|(t, _, _, _)| *t);
    if limit > 0 {
        rows.truncate(limit as usize);
    }

    let listing = rows
        .iter()
        .map(|(t, mgdl, trend, src)| format!("{t}\t{mgdl}\t{trend}\t{src}"))
        .collect::<Vec<_>>()
        .join("\n");
    to_jstring(env, listing)
}

/// Every treatment this phone can open for a followed subject after `since_ms`,
/// as `kind<TAB>millis<TAB>value<TAB>dur<TAB>flag` lines, oldest first.
///
/// **THESE WERE ALWAYS BEING SENT, AND THE READER WAS THROWING THEM AWAY.** The
/// emitter drains eight kinds — `cgm`, `bolus`, `carb`, `tbr`, `extbolus`,
/// `event`, `profile`, `target` — so a granted follower has been decrypting
/// boluses and carbs all along and discarding them one line after opening them,
/// because [`Java_nz_diaswarm_jni_SwarmNative_netGlucose`] filters
/// `kind() == "cgm"`. Nothing about the protocol, the grant or the spec changes
/// here. This returns what was already on the phone.
///
/// **WHAT IS STILL GENUINELY ABSENT, AND ALWAYS WILL BE.** Predictions and
/// loop telemetry. `deviceStatus` and `apsResults` are excluded at the emit
/// boundary by D6 — no clinical content, and the tables most likely to hold
/// something nobody meant to share — and the record vocabulary is closed. So
/// "eventual BG", the loop's own IOB, and its reasoning are not late, they are
/// not coming. A follower that wants IOB must compute it, which is a different
/// decision with a safety argument attached, and is not this function.
///
/// The shape is deliberately one row type rather than four, because the caller
/// draws them on one time axis and the differences are all in two numbers:
///
/// | kind | `value` | `dur` | `flag` |
/// |---|---|---|---|
/// | `bolus` | units | 0 | the bolus type (`NORMAL`, `SMB`, …) |
/// | `carb` | grams | ms, 0 if not extended | empty |
/// | `tbr` | rate | ms | `abs` when U/h, else empty and the rate is a percent |
/// | `extbolus` | units | ms | empty |
///
/// `dur` is milliseconds, per spec §2 — never minutes, because real durations
/// include 36,690 ms and rounding loses what the device had.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_netTreatments<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    store_path: JString<'a>,
    subject: JString<'a>,
    identity_path: JString<'a>,
    purpose: JString<'a>,
    since_ms: jlong,
    limit: jint,
) -> JString<'a> {
    let (Ok(store), Ok(subj), Ok(id), Ok(p)) = (
        env.get_string(&store_path),
        env.get_string(&subject),
        env.get_string(&identity_path),
        env.get_string(&purpose),
    ) else {
        return to_jstring(env, String::new());
    };
    let dir = PathBuf::from(String::from(store)).join(String::from(subj));
    let Some(reader) = load_or_create_identity(Path::new(&String::from(id))) else {
        return to_jstring(env, String::new());
    };
    let Ok(vault) = Vault::open(&dir) else { return to_jstring(env, String::new()) };

    // Bounded by epoch exactly as `netGlucose` is, and for the same reason:
    // this runs on the same two-minute poll and must not open a year.
    let from_epoch = if since_ms > 0 {
        epoch_of(since_ms, vault.offset())
    } else {
        i64::MIN
    };
    let Ok(opened) = vault.read_as_from(&reader, &String::from(p), from_epoch) else {
        return to_jstring(env, String::new());
    };

    let num = |r: &Record, k: &str| r.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let txt = |r: &Record, k: &str| {
        r.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string()
    };

    let mut rows: Vec<(i64, String)> = opened
        .values()
        .flatten()
        .filter(|r| r.t() > since_ms)
        .filter_map(|r| {
            let kind = r.kind();
            let line = match kind {
                "bolus" => format!("bolus\t{}\t{}\t0\t{}", r.t(), num(r, "u"), txt(r, "type")),
                "carb" => format!("carb\t{}\t{}\t{}\t", r.t(), num(r, "g"), num(r, "dur")),
                "tbr" => {
                    // `abs` decides what `rate` MEANS — U/h or a percentage of
                    // basal. Handing the number over without it would put a
                    // "150" on a chart that could be 150% or 150 U/h.
                    let abs = r.get("abs").and_then(|v| v.as_bool()).unwrap_or(false);
                    format!(
                        "tbr\t{}\t{}\t{}\t{}",
                        r.t(),
                        num(r, "rate"),
                        num(r, "dur"),
                        if abs { "abs" } else { "" }
                    )
                }
                "extbolus" => {
                    format!("extbolus\t{}\t{}\t{}\t", r.t(), num(r, "u"), num(r, "dur"))
                }
                _ => return None,
            };
            Some((r.t(), line))
        })
        .collect();

    // Same overlap as `netGlucose`: segments legitimately repeat, and a bolus
    // drawn twice is a bolus that looks like two.
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    rows.dedup_by(|a, b| a.1 == b.1);
    if limit > 0 {
        rows.truncate(limit as usize);
    }

    let listing = rows.iter().map(|(_, l)| l.clone()).collect::<Vec<_>>().join("\n");
    to_jstring(env, listing)
}

/// The newest profile a followed subject has published, as its canonical JSON,
/// or empty if this reader can open none.
///
/// **A FOLLOWER'S GRAPH NEEDS THE SUBJECT'S PROFILE, NOT A MADE-UP ONE.** AAPS
/// draws glucose in the units, and against the target band, of the profile in
/// force — with no profile it draws nothing at all, and with a locally invented
/// one it would draw somebody else's blood against a stranger's targets. The
/// subject already publishes exactly this: §2 requires `profile` records to
/// carry basal, ISF, IC and target blocks normalised to mg/dL, precisely so a
/// consumer can say what the loop was trying to do.
///
/// Only the newest is returned. A follower wants the profile in force now; the
/// history of profile changes is in the vault for anyone assembling one.
///
/// Unbounded on purpose, unlike `netGlucose`: profile records are rare — 19 in
/// this subject's 74 days — and the one in force may have been published long
/// before the window a follower is watching. Bounding this to recent epochs
/// would silently leave a long-settled profile unfindable.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_netProfile<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    store_path: JString<'a>,
    subject: JString<'a>,
    identity_path: JString<'a>,
    purpose: JString<'a>,
) -> JString<'a> {
    let (Ok(store), Ok(subj), Ok(id), Ok(p)) = (
        env.get_string(&store_path),
        env.get_string(&subject),
        env.get_string(&identity_path),
        env.get_string(&purpose),
    ) else {
        return to_jstring(env, String::new());
    };
    let dir = PathBuf::from(String::from(store)).join(String::from(subj));
    let Some(reader) = load_or_create_identity(Path::new(&String::from(id))) else {
        return to_jstring(env, String::new());
    };
    let Ok(vault) = Vault::open(&dir) else { return to_jstring(env, String::new()) };
    let Ok(opened) = vault.read_as(&reader, &String::from(p)) else {
        return to_jstring(env, String::new());
    };
    let newest = opened
        .values()
        .flatten()
        .filter(|r| r.kind() == "profile")
        .max_by_key(|r| r.t());
    match newest {
        Some(r) => to_jstring(env, r.to_canonical_json()),
        None => to_jstring(env, String::new()),
    }
}

/// Who this subject has granted, one per line: `reader<TAB>purpose`.
///
/// Read from the vault's private book, which is the only thing that can put a
/// name to a tag — the grant log deliberately cannot (D13). Empty when nobody
/// has been granted, which is indistinguishable here from a vault that does
/// not exist; both mean "nothing to show", and the caller knows which.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_vaultReaders<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    vault_path: JString<'a>,
) -> JString<'a> {
    let Ok(v) = env.get_string(&vault_path) else { return to_jstring(env, String::new()) };
    let Ok(vault) = Vault::open(Path::new(&String::from(v))) else {
        return to_jstring(env, String::new());
    };
    let listing = vault
        .readers()
        .unwrap_or_default()
        .iter()
        .map(|k| format!("{}\t{}", k.reader, k.purpose))
        .collect::<Vec<_>>()
        .join("\n");
    to_jstring(env, listing)
}

/// The one string a subject hands to someone they want to share with.
///
/// Empty on failure — the caller has a subject key and an endpoint id to hand
/// and can say what is missing more usefully than a code could.
///
/// Composed here rather than in Kotlin so that the phone and the CLI cannot
/// drift into two formats that look alike and are not.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_inviteFor<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    subject: JString<'a>,
    endpoint: JString<'a>,
    purpose: JString<'a>,
    keys: JString<'a>,
) -> JString<'a> {
    let (Ok(s), Ok(e), Ok(p)) =
        (env.get_string(&subject), env.get_string(&endpoint), env.get_string(&purpose))
    else {
        return to_jstring(env, String::new());
    };
    // **EMPTY KEYS MEANS A v2 INVITE, AND THAT IS THE MIGRATION STORY.** A
    // phone with no keys vault hands out exactly what it handed out before, so
    // every build already installed can still read it. Only a phone that can
    // actually grant on the keys vault emits something older builds refuse —
    // which they then say clearly, rather than half-working.
    let keys = env.get_string(&keys).map(String::from).unwrap_or_default();
    match diaswarm_core::invite::Invite::new(&String::from(s), &String::from(e), &String::from(p))
        .and_then(|inv| inv.with_keys(&keys))
    {
        Ok(inv) => to_jstring(env, inv.encode()),
        Err(_) => to_jstring(env, String::new()),
    }
}

/// Read an invite, returning `subject\tendpoint\tpurpose`, or empty if it is
/// not one. Lets the phone accept an invite from another subject later.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_inviteParse<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    text: JString<'a>,
) -> JString<'a> {
    let Ok(t) = env.get_string(&text) else { return to_jstring(env, String::new()) };
    match diaswarm_core::invite::Invite::parse(&String::from(t)) {
        // A FIFTH FIELD, EMPTY ON A v1 OR v2 INVITE. It is what lets the
        // subject grant this reader on the keys vault as well as the old one —
        // without it a scan grants half of what the invite offers.
        Ok(i) => to_jstring(
            env,
            format!("{}\t{}\t{}\t{}\t{}", i.subject, i.endpoint, i.purpose, i.relay, i.keys),
        ),
        Err(_) => to_jstring(env, String::new()),
    }
}

/// Bring every granted reader's wraps up to date. Returns the number written,
/// or a negative code.
///
/// Sealing does this on every write, so on a healthy vault this returns 0. It
/// exists for the vault that is not healthy: one written by a build that only
/// wrapped at grant time, whose readers are holding segments they cannot open.
/// Called at start so that repairs itself rather than waiting for someone to
/// notice a follower has gone quiet.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_vaultRewrap<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    vault_path: JString<'a>,
) -> jlong {
    let Ok(v) = env.get_string(&vault_path) else { return -1 };
    let Ok(vault) = Vault::open(Path::new(&String::from(v))) else { return -3 };
    match vault.rewrap() {
        Ok(done) => done.written as jlong,
        Err(_) => -4,
    }
}

// ---------------------------------------------------------------------------
// Granting, from the phone
// ---------------------------------------------------------------------------
//
// Until this existed, sharing meant pulling a 3 MB vault over adb, running a
// Rust CLI on a laptop and pushing it back. Nobody does that, which made the
// flagship — hand your partner a key, take it back — something the design could
// describe and not perform.

fn parse_reader(hexed: &str) -> Option<[u8; 32]> {
    let bytes = diaswarm_core::vault::unhex(hexed).ok()?;
    bytes.try_into().ok()
}

/// Grant a reader, and publish the wraps they are now entitled to.
///
/// Returns the number of wraps written, or a negative code: -1 bad arguments,
/// -2 identity unavailable, -3 vault unavailable, -4 the grant failed.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_vaultGrant<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    vault_path: JString<'a>,
    identity_path: JString<'a>,
    reader_pub: JString<'a>,
    purpose: JString<'a>,
) -> jlong {
    let (Ok(v), Ok(i), Ok(r), Ok(p)) = (
        env.get_string(&vault_path),
        env.get_string(&identity_path),
        env.get_string(&reader_pub),
        env.get_string(&purpose),
    ) else {
        return -1;
    };
    let (v, i, r, p) = (String::from(v), String::from(i), String::from(r), String::from(p));
    let Some(reader) = parse_reader(&r) else { return -1 };
    let Some(subject) = load_or_create_identity(Path::new(&i)) else { return -2 };
    let Ok(vault) = Vault::open(Path::new(&v)) else { return -3 };

    // From segment 0: a first grant hands over the whole record, which is what
    // "share my data with my partner" means. Narrowing is the caller's job and
    // wants a UI, not a default.
    if vault.record_grant(&subject, &reader, &p, "grant", 0).is_err() {
        return -4;
    }
    match vault.publish_wraps(&subject, &reader, &p) {
        Ok(n) => n as jlong,
        Err(_) => -4,
    }
}

/// Withdraw, immediately. Returns the segment it takes effect from.
///
/// Rotates first, so everything written after this lands in a segment the
/// reader is not wrapped for. Not "at the next day boundary" — at UTC+12 that
/// could have been most of a day.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_vaultRevoke<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    vault_path: JString<'a>,
    identity_path: JString<'a>,
    reader_pub: JString<'a>,
    purpose: JString<'a>,
) -> jlong {
    let (Ok(v), Ok(i), Ok(r), Ok(p)) = (
        env.get_string(&vault_path),
        env.get_string(&identity_path),
        env.get_string(&reader_pub),
        env.get_string(&purpose),
    ) else {
        return -1;
    };
    let (v, i, r, p) = (String::from(v), String::from(i), String::from(r), String::from(p));
    let Some(reader) = parse_reader(&r) else { return -1 };
    let Some(subject) = load_or_create_identity(Path::new(&i)) else { return -2 };
    let Ok(vault) = Vault::open(Path::new(&v)) else { return -3 };
    match vault.revoke(&subject, &reader, &p) {
        Ok(from) => from as jlong,
        Err(_) => -4,
    }
}

// ---------------------------------------------------------------------------
// Serving, from the phone
// ---------------------------------------------------------------------------






fn to_jstring(env: JNIEnv<'_>, s: String) -> JString<'_> {
    env.new_string(s).unwrap_or_else(|_| unsafe { JString::from_raw(std::ptr::null_mut()) })
}

#[cfg(test)]
mod identity_tests {
    /// THE PHONE MUST KEEP THE ID IT ALREADY HAD.
    ///
    /// The node key on disk was written for iroh and is now handed to p2panda.
    /// If the two derive different public keys from the same 32 bytes, every
    /// phone silently becomes a new peer on upgrade: its share of the pool
    /// reshuffles, and anyone holding its old address finds nobody.
    #[test]
    fn the_same_node_key_gives_the_same_endpoint_id() {
        let raw: [u8; 32] = iroh::SecretKey::generate().to_bytes();

        let iroh_id = iroh::SecretKey::from_bytes(&raw).public().to_string();
        let panda_id = p2panda_core::SigningKey::from_bytes(&raw).verifying_key().to_string();

        assert_eq!(
            iroh_id, panda_id,
            "iroh and p2panda derive different ids from one key — upgrading would \
             change every phone's identity"
        );
    }
}

// ---------------------------------------------------------------------------
// The spaces vault
// ---------------------------------------------------------------------------
//
// A SECOND VAULT, ALONGSIDE THE FIRST, AND NOT WIRED TO ANYTHING YET.
//
// `diaswarm-spaces` replaces the hand-composed sealing construction above with
// p2panda's own key layer (D20), and `diaswarm-net::replicate` replaces the
// vault protocol with log sync (D21). Both are measured — on this subject's
// real history, and on this phone — and neither has ever run inside AAPS.
//
// So they ship switched off, behind their own JNI entry points, next to the
// ones that work. The plugin decides which to call; nothing here changes what
// a phone does until it does. That is the same posture the plugin itself takes
// (SECURITY.md: it ships disabled and cannot dose), for the same reason: the
// failure mode being guarded against is not a bad reading, it is an app that
// will not start on a phone driving an insulin pump.
//
// The old vault stays until the differential test has been run against a
// migrated device, not merely against a copy of its database.

/// An open spaces vault and the runtime it needs.
///
/// The runtime is owned here because `diaswarm-spaces` is async and JNI is not.
/// One per vault rather than one shared: a handle that outlives its runtime is
/// a use-after-free, and the lifetimes are easier to see when they are the same
/// object.
struct SpacesVault {
    runtime: tokio::runtime::Runtime,
    vault: diaswarm_spaces::Vault,
}

/// Open, or create, the spaces vault under a directory. Returns a handle, or 0.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_spacesOpen<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    dir: JString<'a>,
    offset_ms: jlong,
) -> jlong {
    let Ok(dir) = env.get_string(&dir) else { return 0 };
    let dir = PathBuf::from(String::from(dir));

    let Ok(runtime) = tokio::runtime::Runtime::new() else { return 0 };
    let Ok(vault) = runtime.block_on(diaswarm_spaces::Vault::open(dir, offset_ms)) else {
        return 0;
    };
    Box::into_raw(Box::new(SpacesVault { runtime, vault })) as jlong
}

/// Close it. Safe to call with 0.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_spacesClose<'a>(
    _env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
) {
    if handle == 0 {
        return;
    }
    // Dropping the runtime stops its threads; dropping the vault closes the
    // store. Order matters only in that both must happen, which `Box` does.
    drop(unsafe { Box::from_raw(handle as *mut SpacesVault) });
}

/// The subject's public key, hex. Empty on failure.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_spacesSubject<'a>(
    env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
) -> JString<'a> {
    if handle == 0 {
        return to_jstring(env, String::new());
    }
    let v = unsafe { &*(handle as *const SpacesVault) };
    to_jstring(env, v.vault.subject().to_hex())
}

/// Seal a batch of canonical records. Returns how many were sealed, or < 0.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_spacesSeal<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
    ndjson: JString<'a>,
) -> jlong {
    if handle == 0 {
        return -1;
    }
    let Ok(body) = env.get_string(&ndjson) else { return -2 };
    let body = String::from(body);
    let v = unsafe { &mut *(handle as *mut SpacesVault) };

    let records: Vec<Record> = body
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| Record::from_json(l).ok())
        .map(Record::normalise)
        .collect();

    match v.runtime.block_on(v.vault.seal(&records)) {
        Ok(_) => records.len() as jlong,
        Err(_) => -3,
    }
}

/// Grant a reader. `history` decides whether it reaches back (D20).
///
/// Returns 0, or < 0. The reader must already be known to this vault — on a
/// phone that is what scanning an invite does.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_spacesGrant<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
    reader_hex: JString<'a>,
    history: jboolean,
) -> jlong {
    if handle == 0 {
        return -1;
    }
    let Ok(who) = env.get_string(&reader_hex) else { return -2 };
    let Some(reader) = verifying_key(&String::from(who)) else { return -3 };
    let v = unsafe { &mut *(handle as *mut SpacesVault) };

    let reach = if history != 0 {
        diaswarm_spaces::Reach::Everything
    } else {
        diaswarm_spaces::Reach::FromNow
    };
    match v.runtime.block_on(v.vault.grant(reader, reach)) {
        Ok(_) => 0,
        Err(_) => -4,
    }
}

/// Withdraw a reader's access, from the next thing sealed onward.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_spacesRevoke<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
    reader_hex: JString<'a>,
) -> jlong {
    if handle == 0 {
        return -1;
    }
    let Ok(who) = env.get_string(&reader_hex) else { return -2 };
    let Some(reader) = verifying_key(&String::from(who)) else { return -3 };
    let v = unsafe { &mut *(handle as *mut SpacesVault) };
    match v.runtime.block_on(v.vault.revoke(reader)) {
        Ok(_) => 0,
        Err(_) => -4,
    }
}

/// A one-line summary, for a log line or a status row.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_spacesStatus<'a>(
    env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
) -> JString<'a> {
    if handle == 0 {
        return to_jstring(env, String::from("no vault"));
    }
    let v = unsafe { &*(handle as *const SpacesVault) };
    let readers = v.runtime.block_on(v.vault.reader_ids()).map(|r| r.len()).unwrap_or(0);
    let summary = format!("{} windows, {readers} readers", v.vault.windows());
    to_jstring(env, summary)
}

/// Parse a hex public key. `None` rather than a panic on anything unexpected:
/// this comes from a QR code somebody photographed.
fn verifying_key(hex: &str) -> Option<p2panda_core::VerifyingKey> {
    let hex = hex.trim();
    if hex.len() != 64 {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    p2panda_core::VerifyingKey::from_bytes(&bytes).ok()
}

// ---------------------------------------------------------------------------
// The keys vault, and what shadow mode is actually for
// ---------------------------------------------------------------------------
//
// SHADOW MODE SHADOWS THE VAULT THAT IS GOING TO SHIP, AND UNTIL NOW IT DID
// NOT. It sealed into `diaswarm-spaces`, which D26 decided against: spaces
// cannot express a follower who reads only the last day, because its
// application messages chain to their space's previous tips. Shadowing it was
// measuring the thing that is not going to happen.
//
// AND IT COMPARED NOTHING. Four places said it logs "whether they agree" — the
// preference summary a user reads included — and the code added up how many
// records the shadow vault sealed and logged that number beside an unrelated
// status line from the other vault. A shadow vault that silently kept four days
// out of five read exactly like one working perfectly, which is the failure
// shape this project keeps being bitten by, sitting inside the component whose
// job is catching it.
//
// SO "AGREE" MEANS SOMETHING DIFFERENT HERE, AND THE CHANGE IS DELIBERATE. Not
// "the two vaults agree with each other" — the old vault is the thing being
// replaced and is itself fallible — but "the new vault gives back exactly the
// records it was handed". That is ground truth rather than a second opinion,
// it is what `diaswarm-keys/tests/differential.rs` already asserts offline
// against 74 days of real history, and it is a strictly stronger claim: two
// vaults can agree by losing the same record.
//
// It is also what found the bug that made this worth doing. Asking what shadow
// mode would have to compare is what exposed `Vault::seal` replacing a day
// rather than appending to it — because the comparison is between a vault and
// what it was given, and the first thing to check is whether the vault kept it.

/// An open keys vault.
///
/// **THE TAG IS NOT DECORATION.** Every vault handle crosses JNI as a bare
/// `jlong`, so nothing in the type system stops Kotlin passing a keys handle to
/// `spacesClose` — and `SwarmNative`'s own comment says why that matters: "two
/// native handle types reachable from Kotlin is a crash waiting for whoever
/// passes the wrong one". It happened during this very change: the close in
/// `sealPending`'s `finally` was left as `spacesClose`, which would have freed
/// this struct as a `SpacesVault` on a phone driving an insulin pump.
///
/// A leading magic word turns that from undefined behaviour into a refusal with
/// a log line. It cannot catch a *stale* pointer, only a wrongly-typed one, and
/// that is still the difference between a bug and a memory-safety incident.
const KEYS_TAG: u64 = 0x6b65_7973_7661_756c; // "keysvaul"

struct KeysVault {
    tag: u64,
    /// **THE STORE IS NOT OPTIONAL, BECAUSE A GRANT IS NOT A LOCAL EVENT.**
    /// `Vault::grant` returns a message; until `wire::publish_control` has put
    /// it in the control log, no reader can ever receive it and no peer can
    /// replicate it. A vault without a store can seal and nothing else.
    ///
    /// **BORROWED FROM THE POOL, NOT OPENED HERE.** `p2panda-store` builds its
    /// pool with `max_connections(1)` and no busy timeout, so a second pool on
    /// the same file would make sealing and replication take turns failing. The
    /// runtime is borrowed for the same reason in reverse: a handle that is
    /// opened and closed within a pass must not own the runtime a long-lived
    /// sync subscription is running on.
    handle: tokio::runtime::Handle,
    store: diaswarm_keys::SqliteStore,
    signing: p2panda_core::SigningKey,
    vault: diaswarm_keys::Vault,
}

/// Borrow a handle, or `None` if it is not one of ours.
fn keys_vault<'h>(handle: jlong) -> Option<&'h mut KeysVault> {
    if handle == 0 {
        return None;
    }
    let v = unsafe { &mut *(handle as *mut KeysVault) };
    if v.tag != KEYS_TAG { None } else { Some(v) }
}

/// Open, or create, the keys vault under a directory. Returns a handle, or 0.
///
/// **IT REUSES THE PHONE'S EXISTING IDENTITY**, the same 32-byte Ed25519 seed
/// the core vault signs its grant log with, so a subject is one subject
/// whichever vault is asked. Minting a second key would make the shadow vault a
/// different person — every invite, every grant and every pool bucket would be
/// somebody else's, and the comparison would be against a stranger's history.
///
/// The encryption identity cannot come from that seed: `SecretKey::from_bytes`
/// is `test_utils` only, so `Vault::key_bundle` generates one and the manager
/// state is persisted in `group.cbor`. That happens once, on the first open.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_keysOpen<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    pool: jlong,
    dir: JString<'a>,
    identity_path: JString<'a>,
    offset_ms: jlong,
) -> jlong {
    if pool == 0 {
        return 0;
    }
    let pooled = unsafe { &*(pool as *const Pooled) };
    let Some(store) = pooled.keys_store.clone() else { return 0 };
    let handle = pooled.runtime.handle().clone();

    let (Ok(dir), Ok(id_s)) = (env.get_string(&dir), env.get_string(&identity_path)) else {
        return 0;
    };
    let dir = PathBuf::from(String::from(dir));
    let Some(identity) = load_or_create_identity(&PathBuf::from(String::from(id_s))) else {
        return 0;
    };

    let signing = p2panda_core::SigningKey::from_bytes(&identity.signing.to_bytes());
    if std::fs::create_dir_all(&dir).is_err() {
        return 0;
    }
    let Ok(mut vault) = diaswarm_keys::Vault::open(&dir, offset_ms, &signing) else {
        return 0;
    };

    // No group on disk means this is the first open. Creating one generates the
    // encryption identity and writes it out; a vault that came back from a
    // reboot without it would be a new member and every grant to it would be
    // dead.
    if !vault.is_welcomed() {
        let rng = diaswarm_keys::Rng::default();
        let Ok((manager, _bundle)) = diaswarm_keys::Vault::key_bundle(&rng) else { return 0 };
        let Ok(create) = vault.create(manager) else { return 0 };
        // **PUBLISHED, NOT DISCARDED.** The control log is D13's grant log here,
        // and one that starts at the first grant cannot show that nothing came
        // before it. An earlier version of this dropped the message on the
        // floor, so every vault made by it has a log beginning mid-history.
        if handle
            .block_on(diaswarm_keys::wire::publish_control(&store, &signing, &create))
            .is_err()
        {
            return 0;
        }
    }
    Box::into_raw(Box::new(KeysVault { tag: KEYS_TAG, handle, store, signing, vault })) as jlong
}

/// Close it. Safe to call with 0.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_keysClose<'a>(
    _env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
) {
    if keys_vault(handle).is_none() {
        return;
    }
    drop(unsafe { Box::from_raw(handle as *mut KeysVault) });
}

/// The subject's public key, hex. Empty on failure.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_keysSubject<'a>(
    env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
) -> JString<'a> {
    let Some(v) = keys_vault(handle) else {
        return to_jstring(env, String::new());
    };
    // **THE LOG AUTHOR, NOT THE MEMBER TAG.** This used to return
    // `GrantTag::own(..)` through its `Display`, which truncates to eight bytes
    // and names nothing anybody can fetch by. Every caller — `keysCarry`,
    // `control_from`, `segments_tail` — wants the key the logs are under.
    to_jstring(env, v.vault.signer().to_hex())
}

/// Seal a batch into one epoch, read it back, and say whether it survived.
///
/// **THE READ-BACK IS THE WHOLE POINT.** Sealing returns a count of what it was
/// given, which is a statement about the argument and not about the vault. This
/// re-opens the segment from disk afterwards and checks that every record
/// handed in comes back out, byte for byte in canonical form.
///
/// Returns a single line, always parseable, never an exception:
///
/// ```text
/// ok epoch=20342 given=37 held=1586 missing=0 lost=0
/// ```
///
/// * `given` — records in this batch;
/// * `held` — records the epoch holds afterwards, which is larger because a day
///   arrives in pieces;
/// * `missing` — records handed in that did not come back. **Must be 0.** This
///   is the number the whole feature exists to produce, and the one that was
///   never computed.
/// * `lost` — segments that would not open or lines that would not parse, from
///   [`diaswarm_keys::Skipped::lost`]. Also must be 0.
///
/// A failure comes back as `error <what>` rather than a negative number,
/// because a shadow that fails is a log line and the log line should say what
/// happened.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_keysSealChecked<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
    epoch: jlong,
    ndjson: JString<'a>,
) -> JString<'a> {
    let Ok(body) = env.get_string(&ndjson) else {
        return to_jstring(env, "error bad-argument".to_string());
    };
    let body = String::from(body);
    let Some(v) = keys_vault(handle) else {
        return to_jstring(env, "error no-vault".to_string());
    };

    to_jstring(env, seal_checked(&mut v.vault, epoch, &body))
}

/// A followed subject's readings out of the keys vault, in `netGlucose`'s shape.
///
/// **THE SAME ROWS, SO NOTHING ABOVE HAS TO CHANGE.** The follower's chart,
/// its parsing and its units all consume `millis<TAB>mgdl<TAB>trend<TAB>src`
/// from `netGlucose`. Returning anything else here would mean rewriting the
/// screen to find out whether the vault works, and the screen is not what is
/// being tested.
///
/// **BOUNDED BY DAYS, NOT BY EPOCH ARITHMETIC.** `netGlucose` converts
/// `since_ms` to an epoch and reads from there; this asks the log for the last
/// N entries, which the store satisfies by sequence number rather than by
/// reading everything and filtering. A follower wanting six hours asks for two
/// days and filters — the extra day is the epoch boundary, not slack.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_keysGlucose<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
    joined_dir: JString<'a>,
    subject_keys: JString<'a>,
    purpose: JString<'a>,
    since_ms: jlong,
    limit: jlong,
) -> JString<'a> {
    let (Ok(joined), Ok(keys), Ok(purpose)) =
        (env.get_string(&joined_dir), env.get_string(&subject_keys), env.get_string(&purpose))
    else {
        return to_jstring(env, String::new());
    };
    let (joined, keys, purpose) = (String::from(joined), String::from(keys), String::from(purpose));
    let Some(v) = keys_vault(handle) else { return to_jstring(env, String::new()) };
    let offset = v.vault.offset();
    let own_dir = v.vault.root().to_path_buf();

    // Two days covers any window a follower shows, because `since_ms` falls
    // inside an epoch and the epoch containing it has to be read whole.
    let tail = if since_ms > 0 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(since_ms);
        let days = (now.saturating_sub(since_ms)) / 86_400_000 + 2;
        days.max(2) as u64
    } else {
        0
    };

    let out = follow_read(
        &own_dir,
        Path::new(&joined),
        &v.store,
        &v.handle,
        &v.signing,
        &keys,
        &purpose,
        tail,
        i64::MIN,
        offset,
    );
    let Some(body) = out.strip_prefix("ok ").and_then(|rest| rest.split_once('\n')) else {
        // An error is empty here rather than a message: the caller is a chart,
        // and `netGlucose` answers the same way when it can open nothing.
        return to_jstring(env, String::new());
    };

    let mut rows: Vec<(i64, f64, String, String)> = body
        .1
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| Record::from_json(l).ok())
        .filter(|r| r.kind() == "cgm")
        .filter(|r| r.t() > since_ms)
        .filter_map(|r| {
            Some((
                r.t(),
                r.get("mgdl")?.as_f64()?,
                r.get("trend").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                r.get("src").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            ))
        })
        .collect();
    rows.sort_by_key(|(t, _, _, _)| *t);
    rows.dedup_by_key(|(t, _, _, _)| *t);
    if limit > 0 {
        rows.truncate(limit as usize);
    }

    to_jstring(
        env,
        rows.iter()
            .map(|(t, mgdl, trend, src)| format!("{t}\t{mgdl}\t{trend}\t{src}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// Carry every keys log this phone should hold: its own, and each it follows.
///
/// **ONE CALL, BECAUSE THE DECODING BELONGS IN RUST.** A follow records the
/// subject's keys identity as it arrived in their invite, and what `carry`
/// needs out of it is the Ed25519 signer. Handing those to Kotlin so it could
/// hand them straight back would put a hex-decode and a CBOR-decode in the
/// language with no types for either.
///
/// **OUR OWN LOG IS CARRIED TOO, AND THAT IS THE PUBLISHING HALF.** A subject
/// that does not announce its own bucket is a subject nobody can replicate
/// from — the pool would hold followers and no source.
///
/// Returns how many were carried, or negative. A follow with no keys identity
/// is skipped rather than failed: it is somebody paired before the field
/// existed, or a subject with no keys vault, and neither is an error.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_keysCarryAll<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    pool: jlong,
    store_path: JString<'a>,
    identity_path: JString<'a>,
) -> jlong {
    if pool == 0 {
        return -1;
    }
    let (Ok(store), Ok(id_s)) = (env.get_string(&store_path), env.get_string(&identity_path))
    else {
        return -2;
    };
    let store = PathBuf::from(String::from(store));
    let Some(identity) = load_or_create_identity(&PathBuf::from(String::from(id_s))) else {
        return -2;
    };
    let pooled = unsafe { &*(pool as *const Pooled) };
    let Some(replicator) = pooled.keys_replicator.as_ref() else { return -3 };

    let members = pooled
        .runtime
        .block_on(pooled.swarm.pool_members())
        .map(|m| m.len())
        .unwrap_or(2)
        .max(2);
    let depth = diaswarm_net::pool::depth_for(members);

    let mut wanted: Vec<String> = Vec::new();
    // Ours: the same Ed25519 key `keysOpen` signs the logs with.
    wanted.push(
        p2panda_core::SigningKey::from_bytes(&identity.signing.to_bytes())
            .verifying_key()
            .to_hex(),
    );
    for follow in diaswarm_net::peer::load_follows(&store).unwrap_or_default() {
        let Some(keys) = follow.keys.as_deref() else { continue };
        let Ok(id) = diaswarm_keys::decode_identity(keys) else { continue };
        wanted.push(id.signer.to_hex());
    }

    let mut carried = 0i64;
    for subject in wanted {
        let topic = diaswarm_net::pool::bucket_topic(
            depth,
            diaswarm_net::pool::bucket_of(&subject, depth),
        );
        if pooled.runtime.block_on(replicator.carry(topic, &subject)).is_ok() {
            carried += 1;
        }
    }
    carried
}

/// This vault's identity as text, for putting in an invite. Empty on failure.
///
/// Both keys: the one its logs are authored under, and the bundle a grant is
/// agreed against. See [`diaswarm_keys::KeysIdentity`] for why neither implies
/// the other.
///
/// **THE HALF OF A PAIRING THE INVITE DOES NOT CARRY YET.** A grant needs both
/// sides' bundles — see [D27](../../docs/decisions.md) — and this is ours.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_keysIdentity<'a>(
    env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
) -> JString<'a> {
    let Some(v) = keys_vault(handle) else {
        return to_jstring(env, String::new());
    };
    match v.vault.identity().and_then(|i| diaswarm_keys::encode_identity(&i)) {
        Ok(text) => to_jstring(env, text),
        Err(_) => to_jstring(env, String::new()),
    }
}

/// Grant a reader, from the bundle they published. Returns their tag, or `error …`.
///
/// **THE TAG IS THE SUBJECT'S PRIVATE NAME FOR THE RELATIONSHIP**, derived from
/// the shared secret, and it is what the subject's own book should file them
/// under. The grant itself names nobody: that is [D13](../../docs/decisions.md),
/// and it is why this returns the tag rather than publishing it.
///
/// The welcome is published to the control log before this returns. A grant
/// that is not in the log has not happened — no reader can receive it and no
/// peer can replicate it.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_keysGrant<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
    reader_bundle: JString<'a>,
    purpose: JString<'a>,
) -> JString<'a> {
    let (Ok(bundle), Ok(purpose)) = (env.get_string(&reader_bundle), env.get_string(&purpose))
    else {
        return to_jstring(env, "error bad-argument".to_string());
    };
    let (bundle, purpose) = (String::from(bundle), String::from(purpose));
    let Some(v) = keys_vault(handle) else {
        return to_jstring(env, "error no-vault".to_string());
    };

    // **WHAT THE INVITE CARRIES IS AN IDENTITY, NOT A BARE BUNDLE**, and this
    // decoded it as a bundle until a real pairing said
    // `missing field identity_key`. The desktop test could not see it: it
    // encodes and decodes with the same pair of functions, so it agrees with
    // itself. The app encodes an identity on one phone and decoded a bundle on
    // the other, and only two devices put those two halves together.
    //
    // A bare bundle is still accepted, because `twokeys` prints one and a
    // person pasting either should get the grant they asked for.
    let bundle = match diaswarm_keys::decode_identity(&bundle) {
        Ok(id) => id.bundle,
        Err(_) => match diaswarm_keys::decode_bundle(&bundle) {
            Ok(b) => b,
            Err(e) => return to_jstring(env, format!("error bundle {e}")),
        },
    };
    let (welcome, tag) = match v.vault.grant(bundle, &purpose) {
        Ok(pair) => pair,
        Err(e) => return to_jstring(env, format!("error grant {e}")),
    };
    if let Err(e) = v.handle.block_on(diaswarm_keys::wire::publish_control(
        &v.store,
        &v.signing,
        &welcome,
    )) {
        return to_jstring(env, format!("error publish {e}"));
    }
    let mut hex = String::with_capacity(64);
    for b in &tag.0 {
        hex.push_str(&format!("{b:02x}"));
    }
    to_jstring(env, hex)
}

/// Withdraw a reader, by the tag [`Java_nz_diaswarm_jni_SwarmNative_keysGrant`]
/// returned. 0 on success, negative otherwise.
///
/// **IT BITES THE DAY IT HAPPENS IN.** Removing a member rotates the group
/// secret, and the current epoch is re-sealed under the new one the next time
/// anything is sealed into it — so a reader revoked at noon does not read the
/// afternoon. Days that were already finished keep their own secret and stay
/// readable, which is what makes this a withdrawal rather than a deletion.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_keysRevoke<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
    tag_hex: JString<'a>,
) -> jlong {
    let Ok(tag_hex) = env.get_string(&tag_hex) else { return -1 };
    let tag_hex = String::from(tag_hex);
    if tag_hex.len() != 64 {
        return -2;
    }
    let mut raw = [0u8; 32];
    for i in 0..32 {
        match u8::from_str_radix(&tag_hex[i * 2..i * 2 + 2], 16) {
            Ok(b) => raw[i] = b,
            Err(_) => return -2,
        }
    }
    let Some(v) = keys_vault(handle) else { return -3 };

    let message = match v.vault.revoke(diaswarm_keys::group::GrantTag(raw)) {
        Ok(m) => m,
        Err(_) => return -4,
    };
    // **THE REVOCATION MUST REACH THE OTHER READERS, NOT JUST THE LOG.** It
    // carries the rotated secret as direct messages to everyone still in the
    // group; a remaining reader that never processes it stops opening days at
    // the moment somebody *else* was revoked, with nothing to say why.
    if v.handle
        .block_on(diaswarm_keys::wire::publish_control(&v.store, &v.signing, &message))
        .is_err()
    {
        return -5;
    }
    0
}

/// The whole of `keysSealChecked`, with no JNI in it.
///
/// Split out so the line the Kotlin parses can be asserted on a desktop. The
/// two halves of that contract are in different languages and are linked by
/// name at load time on a phone — `plugin/README.md` says why that repository
/// layout exists — so the format is exactly the kind of thing that drifts
/// silently. And the Kotlin's failure mode if it drifts is the bad one: a field
/// it cannot find would read as `missing=0`, which is "agrees".
pub fn seal_checked(vault: &mut diaswarm_keys::Vault, epoch: i64, body: &str) -> String {
    let given: Vec<Record> = body
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| Record::from_json(l).ok())
        .map(Record::normalise)
        .collect();

    if let Err(e) = vault.seal(epoch, &given) {
        return format!("error seal {e}");
    }

    // **READ BACK OFF DISK RATHER THAN TRUSTING WHAT WAS JUST WRITTEN.** A seal
    // returns a count of its argument, which is a statement about the caller.
    let (held, skipped) = match vault.read_reporting(epoch) {
        Ok(pair) => pair,
        Err(e) => return format!("error read {e}"),
    };
    let day = held.get(&epoch).cloned().unwrap_or_default();
    let present: std::collections::HashSet<String> =
        day.iter().map(|r| r.to_canonical_json()).collect();
    let missing = given.iter().filter(|r| !present.contains(&r.to_canonical_json())).count();

    format!(
        "ok epoch={epoch} given={} held={} missing={missing} lost={}",
        given.len(),
        day.len(),
        skipped.lost()
    )
}

/// Join a subject's group from a control log somebody else replicated to us.
///
/// **ONE IDENTITY, SEVERAL VAULTS — WHICH IS WHY THIS TAKES TWO DIRECTORIES.**
/// A `Vault` holds exactly one group state, so following three people means
/// three vaults. But the bundle this device published — the one each subject
/// granted against — belongs to *one* key manager, in this device's own vault.
/// A joined vault that minted its own would be a different member to the one
/// that was granted, and would read nothing while looking perfectly healthy: no
/// error, no crash, an empty graph. So `own_dir` supplies the identity and
/// `joined_dir` holds the group.
///
/// **AND THE READER FINDS ITS OWN WELCOME BY TRYING.** Nothing in a control
/// message says in clear who it is for — a grant that announced its recipient
/// would undo [D13](../../docs/decisions.md) — so every message in the log is
/// offered to `join` and the one that opens is ours. `join` refuses anything
/// that does not actually welcome us, which is what makes trying safe.
pub fn join_subject(
    own_dir: &Path,
    joined_dir: &Path,
    store: &diaswarm_keys::SqliteStore,
    runtime: &tokio::runtime::Handle,
    signing: &p2panda_core::SigningKey,
    keys_hex: &str,
    purpose: &str,
    offset_ms: i64,
) -> Result<(diaswarm_keys::Vault, p2panda_core::VerifyingKey), String> {
    // **ONE FIELD, BOTH KEYS.** Taking the author and the bundle separately let
    // a caller pair one subject's log with another's bundle — a mismatch that
    // produces no error, just a reader that never finds a welcome. They arrive
    // together in the invite and they stay together here.
    let identity =
        diaswarm_keys::decode_identity(keys_hex).map_err(|e| format!("identity {e}"))?;
    let subject = &identity.signer;
    let subject_bundle = identity.bundle.clone();

    // The identity this device published, not a fresh one.
    let own = diaswarm_keys::Vault::open(own_dir, offset_ms, signing)
        .map_err(|e| format!("own vault {e}"))?;
    let manager = own.manager_state().map_err(|e| format!("own identity {e}"))?;
    drop(own);

    let control = runtime
        .block_on(diaswarm_keys::wire::control_from(store, subject, None))
        .map_err(|e| format!("control log {e}"))?;
    if control.is_empty() {
        return Err("no grant has arrived yet".to_string());
    }

    let registry = diaswarm_keys::Vault::registry(&[(
        diaswarm_keys::group::GrantTag::own(subject),
        subject_bundle.clone(),
    )])
    .map_err(|e| format!("registry {e}"))?;

    for message in &control {
        let Ok(mut candidate) = diaswarm_keys::Vault::open(joined_dir, offset_ms, signing) else {
            continue;
        };
        if candidate
            .join(manager.clone(), registry.clone(), &subject_bundle, purpose, message)
            .is_ok()
        {
            return Ok((candidate, *subject));
        }
    }
    Err(format!("none of the {} control messages welcome us", control.len()))
}

/// What a joined vault can open, as NDJSON.
///
/// **SEGMENTS COME FROM THE LOG, NOT A DIRECTORY.** A follower's arrive as
/// operation bodies over `p2panda-net` and never touch the filesystem, which is
/// the whole shape of D26 — so this reads them out of the store rather than
/// calling `read_from`. What it cannot open it counts; a short answer must
/// never be a silent one.
///
/// **`tail_days` IS THE FLAGSHIP'S PARAMETER, AND 0 IS NOT.** A parent needs 24
/// hours (D11), and `segments_tail` answers that by sequence number so the
/// store does the skipping — measured flat in `diaswarm-keys/tests/wire.rs` as
/// the log grows. `segments_from` reads the whole log and filters, which is
/// linear in everything the subject ever sealed: the right primitive for a
/// research export and the wrong one for a follower refreshing every two
/// minutes. Passing 0 asks for that linear read deliberately.
pub fn read_followed(
    vault: &diaswarm_keys::Vault,
    store: &diaswarm_keys::SqliteStore,
    runtime: &tokio::runtime::Handle,
    subject: &p2panda_core::VerifyingKey,
    tail_days: u64,
    from_epoch: i64,
) -> Result<(String, usize, usize), String> {
    let segments = if tail_days > 0 {
        let tail = runtime
            .block_on(diaswarm_keys::wire::segments_tail(store, subject, tail_days))
            .map_err(|e| format!("segments {e}"))?;
        tail.into_iter().filter(|s| s.epoch >= from_epoch).collect()
    } else {
        runtime
            .block_on(diaswarm_keys::wire::segments_from(store, subject, from_epoch))
            .map_err(|e| format!("segments {e}"))?
    };

    let mut out = String::new();
    let mut opened = 0usize;
    let mut unreadable = 0usize;
    for segment in &segments {
        match vault.open_segment(segment) {
            Ok((records, bad)) => {
                opened += 1;
                unreadable += bad;
                for record in records {
                    out.push_str(&record.to_canonical_json());
                    out.push('\n');
                }
            }
            // NOT AN ERROR. A segment sealed before this reader was granted, or
            // after it was revoked, is access control working.
            Err(_) => unreadable += 1,
        }
    }
    Ok((out, opened, unreadable))
}

/// Join if we have not yet, then read everything we can from `from_epoch`.
///
/// **ONE CALL, BECAUSE A FOLLOWER WANTS RECORDS AND NOT A HANDLE.** Joining is
/// a one-off that leaves `group.cbor` behind, so a vault that is already
/// welcomed skips straight to reading — which matters, because `join` replaces
/// the group state and doing it on every refresh would throw away a secret
/// bundle that took a replication round trip to acquire.
///
/// **THE STORE IS THE DEVICE'S, NOT THE SUBJECT'S.** Logs are keyed by author,
/// so one SQLite file holds this device's own log and every subject it follows;
/// that is what `KeysReplicator` writes into and what this reads out of.
///
/// Returns `ok <opened> <unreadable>\n<ndjson>` or `error <what>`. Not a
/// negative number: a follower that read nothing needs to know whether it was
/// never granted, never replicated, or simply has no days yet.
pub fn follow_read(
    own_dir: &Path,
    joined_dir: &Path,
    store: &diaswarm_keys::SqliteStore,
    runtime: &tokio::runtime::Handle,
    signing: &p2panda_core::SigningKey,
    keys_hex: &str,
    purpose: &str,
    tail_days: u64,
    from_epoch: i64,
    offset_ms: i64,
) -> String {
    let Ok(identity) = diaswarm_keys::decode_identity(keys_hex) else {
        return "error identity not-a-keys-identity".to_string();
    };
    let subject = identity.signer;

    let existing = diaswarm_keys::Vault::open(joined_dir, offset_ms, signing)
        .ok()
        .filter(|v| v.is_welcomed());

    let vault = match existing {
        Some(v) => v,
        None => match join_subject(
            own_dir,
            joined_dir,
            store,
            runtime,
            signing,
            keys_hex,
            purpose,
            offset_ms,
        ) {
            Ok((v, _)) => v,
            Err(e) => return format!("error join {e}"),
        },
    };

    match read_followed(&vault, store, runtime, &subject, tail_days, from_epoch) {
        Ok((ndjson, opened, unreadable)) => format!("ok {opened} {unreadable}\n{ndjson}"),
        Err(e) => format!("error read {e}"),
    }
}

/// Join a subject and read what they have shared. See [`follow_read`].
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_keysFollowRead<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
    joined_dir: JString<'a>,
    subject_keys: JString<'a>,
    purpose: JString<'a>,
    tail_days: jlong,
    from_epoch: jlong,
) -> JString<'a> {
    let (Ok(joined), Ok(keys), Ok(purpose)) =
        (env.get_string(&joined_dir), env.get_string(&subject_keys), env.get_string(&purpose))
    else {
        return to_jstring(env, "error bad-argument".to_string());
    };
    let (joined, keys, purpose) = (String::from(joined), String::from(keys), String::from(purpose));

    let Some(v) = keys_vault(handle) else {
        return to_jstring(env, "error no-vault".to_string());
    };
    let offset = v.vault.offset();
    let own_dir = v.vault.root().to_path_buf();

    let out = follow_read(
        &own_dir,
        Path::new(&joined),
        &v.store,
        &v.handle,
        &v.signing,
        &keys,
        &purpose,
        tail_days.max(0) as u64,
        from_epoch,
        offset,
    );
    to_jstring(env, out)
}

#[cfg(test)]
mod shadow_tests {
    use super::seal_checked;

    fn vault(tag: &str) -> (diaswarm_keys::Vault, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "diaswarm-shadow-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let signing = p2panda_core::SigningKey::generate();
        let mut v = diaswarm_keys::Vault::open(&root, 12 * 3_600_000, &signing).unwrap();
        let rng = diaswarm_keys::Rng::default();
        let (manager, _bundle) = diaswarm_keys::Vault::key_bundle(&rng).unwrap();
        v.create(manager).unwrap();
        (v, root)
    }

    fn ndjson(from: i64, n: i64) -> String {
        (0..n)
            .map(|i| {
                diaswarm_core::Record::new(from + i * 300_000, "cgm")
                    .set("mgdl", Some((100.0 + i as f64).into()))
                    .to_canonical_json()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A FOLLOWER JOINS AND READS FROM A LOG SOMEBODY ELSE FILLED.
    ///
    /// **THE WHOLE FOLLOWER PATH, WITHOUT A NETWORK.** Replication is proven
    /// elsewhere — `diaswarm-net`'s `keys_replicate` test, and `twokeys` on two
    /// phones. What this pins is the part the app has to get right afterwards:
    /// find our welcome among messages that name nobody, join with the identity
    /// this device actually published, and open segments that arrived as
    /// operation bodies rather than files.
    ///
    /// The trap it guards is the silent one. A joined vault that minted its own
    /// key manager would be a different member to the one the subject granted,
    /// and would read *nothing* — no error, no crash, an empty graph.
    #[test]
    fn a_follower_joins_from_a_log_and_reads_what_it_was_granted() {
        use diaswarm_keys::{Vault, encode_bundle, wire};

        // A PLAIN TEST WITH ITS OWN RUNTIME, because that is how the JNI calls
        // these: `join_subject` and `read_followed` are synchronous, blocking
        // on a runtime the vault handle owns. Testing them from inside an async
        // context would be testing a shape no caller has.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let store = rt.block_on(async {
            diaswarm_keys::SqliteStoreBuilder::memory().build().await.unwrap()
        });
        let rng = diaswarm_keys::Rng::default();

        // ---- the device that will follow, and the identity it publishes ----
        let me = p2panda_core::SigningKey::generate();
        let own_dir = dir("own");
        let mut own = Vault::open(&own_dir, 12 * 3_600_000, &me).unwrap();
        let (own_mgr, _b) = Vault::key_bundle(&rng).unwrap();
        own.create(own_mgr).unwrap();
        let my_bundle = encode_bundle(&own.my_bundle().unwrap()).unwrap();
        drop(own);

        // ---- the subject, granting that bundle ----
        let subject_key = p2panda_core::SigningKey::generate();
        let mut subject = Vault::open(dir("subject"), 12 * 3_600_000, &subject_key).unwrap();
        let (s_mgr, _sb) = Vault::key_bundle(&rng).unwrap();
        let create = subject.create(s_mgr).unwrap();
        rt.block_on(wire::publish_control(&store, &subject_key, &create)).unwrap();
        let subject_keys =
            diaswarm_keys::encode_identity(&subject.identity().unwrap()).unwrap();

        // A day before the grant, and two after. **ALL THREE OPEN** — see the
        // assertion below and the note on `Vault::grant`.
        let before = subject.seal(30_000, &records(30_000, 4)).unwrap();
        rt.block_on(wire::publish(&store, &subject_key, &before)).unwrap();

        let reader = diaswarm_keys::decode_bundle(&my_bundle).unwrap();
        let (welcome, _tag) = subject.grant(reader, "follow").unwrap();
        rt.block_on(wire::publish_control(&store, &subject_key, &welcome)).unwrap();
        for e in 1..3i64 {
            let seg = subject.seal(30_000 + e, &records(30_000 + e, 4)).unwrap();
            rt.block_on(wire::publish(&store, &subject_key, &seg)).unwrap();
        }

        // ---- and the follower, using nothing but what replication left ----
        let author = subject_key.verifying_key();
        let joined = super::join_subject(
            &own_dir,
            &dir("joined"),
            &store,
            rt.handle(),
            &me,
            &subject_keys,
            "follow",
            12 * 3_600_000,
        )
        .expect("the follower could not join");
        let (joined, _author) = joined;
        assert!(joined.is_welcomed());

        let (ndjson, opened, _unreadable) =
            super::read_followed(&joined, &store, rt.handle(), &author, 0, i64::MIN).expect("read");

        let lines = ndjson.lines().filter(|l| !l.trim().is_empty()).count();

        // **A GRANT REACHES BACK, AND THAT IS NOT AN ACCIDENT OF THIS TEST.**
        // `Group::add` hands the joiner `&y.secrets` — the whole bundle — so a
        // reader granted today can open every day the subject still holds a
        // secret for, including ones sealed before it was ever granted. This
        // asserted 2 when it was written, because that is what the author
        // assumed; the code has always done 3.
        assert_eq!(opened, 3, "a grant should reach back over the whole bundle");
        assert_eq!(lines, 12, "expected 12 records, got {lines}");
    }

    /// A SECOND READ REUSES THE JOIN INSTEAD OF REDOING IT.
    ///
    /// **AND THAT IS CORRECTNESS, NOT SPEED.** `join` replaces the group state
    /// wholesale, so joining again on every refresh would throw away a secret
    /// bundle that took a replication round trip to acquire — and would fail
    /// outright once the welcome has been pruned from the log or the subject
    /// has rotated past it. A follower refreshes every couple of minutes, so
    /// "every refresh" is the normal case, not an edge one.
    #[test]
    fn reading_twice_does_not_rejoin() {
        use diaswarm_keys::{Vault, encode_bundle, wire};

        let rt = tokio::runtime::Runtime::new().unwrap();
        let store = rt.block_on(async {
            diaswarm_keys::SqliteStoreBuilder::memory().build().await.unwrap()
        });
        let rng = diaswarm_keys::Rng::default();

        let me = p2panda_core::SigningKey::generate();
        let own_dir = dir("r-own");
        let mut own = Vault::open(&own_dir, 12 * 3_600_000, &me).unwrap();
        let (own_mgr, _b) = Vault::key_bundle(&rng).unwrap();
        own.create(own_mgr).unwrap();
        let my_bundle = encode_bundle(&own.my_bundle().unwrap()).unwrap();
        drop(own);

        let subject_key = p2panda_core::SigningKey::generate();
        let mut subject = Vault::open(dir("r-subject"), 12 * 3_600_000, &subject_key).unwrap();
        let (s_mgr, _sb) = Vault::key_bundle(&rng).unwrap();
        let create = subject.create(s_mgr).unwrap();
        rt.block_on(wire::publish_control(&store, &subject_key, &create)).unwrap();
        let subject_keys =
            diaswarm_keys::encode_identity(&subject.identity().unwrap()).unwrap();
        let reader = diaswarm_keys::decode_bundle(&my_bundle).unwrap();
        let (welcome, _t) = subject.grant(reader, "follow").unwrap();
        rt.block_on(wire::publish_control(&store, &subject_key, &welcome)).unwrap();
        let seg = subject.seal(31_000, &records(31_000, 5)).unwrap();
        rt.block_on(wire::publish(&store, &subject_key, &seg)).unwrap();

        let author = subject_key.verifying_key();
        let joined_dir = dir("r-joined");
        let call = |from: i64| {
            super::follow_read(
                &own_dir,
                &joined_dir,
                &store,
                rt.handle(),
                &me,
                &subject_keys,
                "follow",
                0,
                from,
                12 * 3_600_000,
            )
        };

        let first = call(i64::MIN);
        assert!(first.starts_with("ok 1 0"), "first read: {}", &first[..first.len().min(40)]);
        assert_eq!(first.lines().skip(1).filter(|l| !l.trim().is_empty()).count(), 5);

        // **THE WELCOME IS NOW GONE FROM THE LOG.** If the second read tried to
        // join again it would find nothing that welcomes it and fail — which is
        // exactly the state a follower is in days after pairing.
        let pruned = rt.block_on(async {
            diaswarm_keys::SqliteStoreBuilder::memory().build().await.unwrap()
        });
        rt.block_on(wire::publish(&pruned, &subject_key, &seg)).unwrap();
        let second = super::follow_read(
            &own_dir,
            &joined_dir,
            &pruned,
            rt.handle(),
            &me,
            &subject_keys,
            "follow",
            0,
            i64::MIN,
            12 * 3_600_000,
        );
        assert!(
            second.starts_with("ok 1 0"),
            "a second read re-joined instead of reusing: {}",
            &second[..second.len().min(60)]
        );
    }

    /// A FOLLOWER ASKING FOR A DAY GETS A DAY, NOT A HISTORY.
    ///
    /// **THE FLAGSHIP'S READ, AND THE ONE THAT HAS TO STAY CHEAP.** A parent
    /// needs 24 hours (D11) while the subject may hold months, and a grant
    /// hands over secrets for all of it — so "what can I open" and "what do I
    /// want" are very different sizes. `segments_tail` asks by sequence number
    /// and lets the store skip; `diaswarm-keys/tests/wire.rs` measures that
    /// staying flat as the log grows. This pins the correctness half: the right
    /// days come back, and only those.
    #[test]
    fn a_tail_read_returns_the_newest_days_and_no_others() {
        use diaswarm_keys::{Vault, encode_bundle, wire};

        let rt = tokio::runtime::Runtime::new().unwrap();
        let store = rt.block_on(async {
            diaswarm_keys::SqliteStoreBuilder::memory().build().await.unwrap()
        });
        let rng = diaswarm_keys::Rng::default();

        let me = p2panda_core::SigningKey::generate();
        let own_dir = dir("t-own");
        let mut own = Vault::open(&own_dir, 12 * 3_600_000, &me).unwrap();
        let (own_mgr, _b) = Vault::key_bundle(&rng).unwrap();
        own.create(own_mgr).unwrap();
        let my_bundle = encode_bundle(&own.my_bundle().unwrap()).unwrap();
        drop(own);

        let subject_key = p2panda_core::SigningKey::generate();
        let mut subject = Vault::open(dir("t-subject"), 12 * 3_600_000, &subject_key).unwrap();
        let (s_mgr, _sb) = Vault::key_bundle(&rng).unwrap();
        let create = subject.create(s_mgr).unwrap();
        rt.block_on(wire::publish_control(&store, &subject_key, &create)).unwrap();
        let subject_keys =
            diaswarm_keys::encode_identity(&subject.identity().unwrap()).unwrap();
        let reader = diaswarm_keys::decode_bundle(&my_bundle).unwrap();
        let (welcome, _t) = subject.grant(reader, "follow").unwrap();
        rt.block_on(wire::publish_control(&store, &subject_key, &welcome)).unwrap();

        // Thirty days sealed, one record each so the counts are unambiguous.
        for e in 0..30i64 {
            let seg = subject.seal(32_000 + e, &records(32_000 + e, 1)).unwrap();
            rt.block_on(wire::publish(&store, &subject_key, &seg)).unwrap();
        }

        let author = subject_key.verifying_key();
        let joined_dir = dir("t-joined");
        let read = |tail: u64| {
            super::follow_read(
                &own_dir,
                &joined_dir,
                &store,
                rt.handle(),
                &me,
                &subject_keys,
                "follow",
                tail,
                i64::MIN,
                12 * 3_600_000,
            )
        };

        let one = read(1);
        assert!(one.starts_with("ok 1 0"), "a one-day tail read {}", &one[..one.len().min(30)]);
        let newest = records(32_029, 1)[0].to_canonical_json();
        assert!(one.contains(&newest), "the newest day was not the one returned");

        let two = read(2);
        assert!(two.starts_with("ok 2 0"), "a two-day tail read {}", &two[..two.len().min(30)]);

        // And 0 still means the whole history, for a research export.
        let all = read(0);
        assert!(all.starts_with("ok 30 0"), "a full read {}", &all[..all.len().min(30)]);
    }

    /// A GRANT TAKES WHAT AN INVITE ACTUALLY CARRIES.
    ///
    /// **THE MISMATCH THAT ONLY TWO PHONES COULD SHOW.** An invite carries a
    /// `KeysIdentity` — signer and bundle together — and the grant path decoded
    /// it as a bare `LongTermKeyBundle`, which fails with
    /// `missing field identity_key`. Every desktop test passed because each one
    /// encodes and decodes with the same pair of functions, so it agrees with
    /// itself. The app encoded an identity on one phone and decoded a bundle on
    /// the other, and nothing put those two halves together until a real
    /// pairing did.
    ///
    /// This pins both forms, because a person pasting either should get the
    /// grant they asked for.
    #[test]
    fn a_grant_accepts_an_identity_or_a_bare_bundle() {
        use diaswarm_keys::{Vault, encode_bundle, encode_identity};

        let rng = diaswarm_keys::Rng::default();
        let key = p2panda_core::SigningKey::generate();
        let mut v = Vault::open(dir("grant-forms"), 12 * 3_600_000, &key).unwrap();
        let (mgr, _b) = Vault::key_bundle(&rng).unwrap();
        v.create(mgr).unwrap();

        let as_identity = encode_identity(&v.identity().unwrap()).unwrap();
        let as_bundle = encode_bundle(&v.my_bundle().unwrap()).unwrap();

        // What `keysGrant` does with each, without the JNI around it.
        assert!(
            diaswarm_keys::decode_identity(&as_identity).is_ok(),
            "an invite's keys field must decode as an identity"
        );
        assert!(
            diaswarm_keys::decode_identity(&as_bundle).is_err(),
            "a bare bundle is not an identity — the fallback exists for this"
        );
        assert!(
            diaswarm_keys::decode_bundle(&as_bundle).is_ok(),
            "twokeys prints a bare bundle and it must still be usable"
        );
    }

    fn dir(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "diaswarm-follow-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn records(epoch: i64, n: i64) -> Vec<diaswarm_core::Record> {
        (0..n)
            .map(|i| {
                diaswarm_core::Record::new(epoch * 86_400_000 + i * 300_000, "cgm")
                    .set("mgdl", Some((100.0 + i as f64).into()))
            })
            .collect()
    }

    /// THE LINE THE KOTLIN PARSES, PINNED.
    ///
    /// Every field the plugin reads by name is here and is a number. A rename
    /// on this side without one on the other turns into "missing 0", which
    /// reads as agreement — so this test is the thing standing between a
    /// refactor and a shadow mode that silently passes.
    #[test]
    fn the_report_says_what_the_plugin_reads() {
        let (mut v, _root) = vault("format");
        let report = seal_checked(&mut v, 20_000, &ndjson(20_000 * 86_400_000, 12));

        assert!(report.starts_with("ok "), "unexpected report: {report}");
        let fields: std::collections::HashMap<&str, &str> = report
            .split(' ')
            .filter_map(|f| f.split_once('='))
            .collect();
        for key in ["epoch", "given", "held", "missing", "lost"] {
            let value = fields.get(key).unwrap_or_else(|| panic!("no {key} in {report}"));
            value.parse::<i64>().unwrap_or_else(|_| panic!("{key} is not a number in {report}"));
        }
        assert_eq!(fields["given"], "12");
        assert_eq!(fields["missing"], "0", "a fresh seal lost records: {report}");
        assert_eq!(fields["lost"], "0");
    }

    /// A DAY ARRIVING IN PIECES STILL REPORTS NOTHING MISSING.
    ///
    /// The shape a phone actually produces: the plugin flushes what has
    /// accumulated since the last cadence, not the whole day. `held` grows and
    /// `missing` stays at zero — and when `seal` replaced the segment instead
    /// of appending to it, this is the test that would have said so.
    #[test]
    fn a_day_sealed_in_pieces_reports_nothing_missing() {
        let (mut v, _root) = vault("pieces");
        let base = 20_001 * 86_400_000;
        let mut held = 0i64;
        for chunk in 0..6 {
            let report = seal_checked(&mut v, 20_001, &ndjson(base + chunk * 12 * 300_000, 12));
            let fields: std::collections::HashMap<&str, &str> =
                report.split(' ').filter_map(|f| f.split_once('=')).collect();
            assert_eq!(fields["missing"], "0", "flush {chunk} lost records: {report}");
            held = fields["held"].parse().unwrap();
        }
        assert_eq!(held, 72, "the day did not accumulate across flushes");
    }
}
