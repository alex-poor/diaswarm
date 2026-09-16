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
use jni::sys::{jint, jlong};
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
    /// Two independent pools on one SQLite file is not a tidiness question but
    /// a writer-contention bug: sealing and replication would take turns
    /// failing, on a phone that may be driving an insulin pump. So there is
    /// exactly one, shared by cloning the handle — and it belongs to the pool
    /// because the pool is the long-lived object. Every other keys handle is
    /// opened and closed within a pass.
    ///
    /// **THIS USED TO SAY `p2panda-store` BUILDS ITS POOL WITH
    /// `max_connections(1)`. IT DOES NOT.** `SqliteStoreBuilder::default()` is
    /// `min_connections: 3, max_connections: 16`; only `memory()` sets one. It
    /// also sets no pragmas at all — no `cache_size`, no `journal_mode`. So
    /// this store and the address book between them hold up to thirty-two
    /// connections, each an OS thread with its own page cache, and a heap
    /// profile on 2026-09-15 put **85% of steady-state allocation in that page
    /// cache** at about 14 MB a minute. The conclusion above survives the
    /// correction; the reason given for it did not.
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
                // A CAPPED PAGE CACHE — see `open_bounded_store`. An
                // uncapped one is what put 85% of this app's steady-state
                // allocation into SQLite page cache.
                let store = diaswarm_keys::open_bounded_store(&url)
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

/// Android says the network changed; tell iroh, which cannot see it itself.
///
/// **THE MISSING HALF OF A 71-MINUTE OUTAGE.** `netwatch` ships a deliberately
/// empty route monitor on Android — "Very sad monitor. Android doesn't allow us
/// to do this" — so nothing native ever learns that wifi became mobile data.
/// iroh's own documentation says Java has to tell it. This is where Java tells
/// it. See `Swarm::network_changed`.
///
/// Returns 1 if the notification was delivered, 0 if there is no swarm to
/// notify. Cheap enough to call on every `ConnectivityManager` callback;
/// upstream says calling it needlessly does no harm.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_swarmNetworkChanged(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jlong {
    if handle == 0 {
        return 0;
    }
    let pooled = unsafe { &*(handle as *const Pooled) };
    pooled.runtime.block_on(pooled.swarm.network_changed());
    1
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
    // **RELAY STATE RIDES ALONG**, because a pool pass is the one thing both
    // apps already run on a cadence and already log. A node whose relay
    // connection has gone is unreachable from every other network while looking
    // perfectly healthy from its own side — the loop phone was found in exactly
    // that state after a night, twelve hours into a process that had connected
    // fine at startup. An outage nobody can see is one nobody fixes.
    let relay = pooled.runtime.block_on(pooled.swarm.relay_state());
    match pooled.runtime.block_on(pooled.swarm.tick_and_adopt(take)) {
        Ok((r, adopted)) => to_jstring(
            env,
            format!(
                "{}\t{}\t{}\t{}\t{}\trelay={relay}",
                r.pool,
                r.buckets,
                r.held,
                r.wanted.len(),
                adopted
            ),
        ),
        Err(_) => to_jstring(env, format!("\t\t\t\t\trelay={relay}")),
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


    let mut rows: Vec<(i64, String)> = opened
        .values()
        .flatten()
        .filter(|r| r.t() > since_ms)
        .filter_map(|r| treatment_line(r).map(|line| (r.t(), line)))
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
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_netTempTarget<'a>(
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
        .filter(|r| r.kind() == "target")
        .max_by_key(|r| r.t());
    match newest {
        Some(r) => to_jstring(env, r.to_canonical_json()),
        None => to_jstring(env, String::new()),
    }
}

/// The subject's newest temporary target, as canonical JSON, or empty.
///
/// **THE TARGET ON SCREEN IS A CLINICAL STATEMENT ATTRIBUTED TO THEM**, and
/// until now it came only from the profile. A temporary target — exercise,
/// eating soon, a hypo — replaces it for as long as it runs, so the screen was
/// showing the profile's band and calling it theirs while the loop was aiming
/// somewhere else entirely. The emitter has always published these; nothing
/// read them.
///
/// Newest wins, and that also handles cancelling: AAPS calls a temporary target
/// off by writing another one with zero duration, so no special case is needed.
/// Whether it is still running is the caller's arithmetic, because only the
/// caller knows what time it is on the phone doing the asking.
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
    handle: JString<'a>,
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
    // **AN EMPTY HANDLE KEEPS THE OLDER SHAPE, for the same reason empty keys
    // does.** A subject who has not named themselves emits exactly what they
    // emitted before, so nothing already scanned stops working.
    let handle = env.get_string(&handle).map(String::from).unwrap_or_default();
    match diaswarm_core::invite::Invite::new(&String::from(s), &String::from(e), &String::from(p))
        .and_then(|inv| inv.with_keys(&keys))
        .and_then(|inv| inv.with_handle(&handle))
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
        //
        // AND A SIXTH: the subject's chosen name, empty before v4. The caller
        // stores it once, at pairing, and shows it thereafter — it is a label,
        // not proof of who sent the invite. See `Invite::handle`.
        Ok(i) => to_jstring(
            env,
            format!(
                "{}\t{}\t{}\t{}\t{}\t{}",
                i.subject, i.endpoint, i.purpose, i.relay, i.keys, i.handle
            ),
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

/// What this subject's private book says a reader's keys identity is, or empty.
///
/// **SO THAT ONE WITHDRAWAL CAN REACH BOTH VAULTS.** The caller names a reader
/// the only way a person can — by the key in their invite — and this answers
/// with the other half, which `keysRevokeReader` turns back into the member to
/// remove. Empty means this subject has never seen a keys identity for them:
/// an older reader who has not handed over yet, or somebody whose app has no
/// keys vault. Not an error, and the core withdrawal still stands on its own.
///
/// It reads `readers.json`, which never leaves the phone — see the note at the
/// top of `diaswarm-core::vault`.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_vaultReaderKeys<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    vault_path: JString<'a>,
    reader_pub: JString<'a>,
    purpose: JString<'a>,
) -> JString<'a> {
    let (Ok(v), Ok(r), Ok(p)) =
        (env.get_string(&vault_path), env.get_string(&reader_pub), env.get_string(&purpose))
    else {
        return to_jstring(env, String::new());
    };
    let (v, r, p) = (String::from(v), String::from(r).to_ascii_lowercase(), String::from(p));
    let Ok(vault) = Vault::open(Path::new(&v)) else { return to_jstring(env, String::new()) };
    let Ok(book) = vault.readers() else { return to_jstring(env, String::new()) };
    let found = book
        .into_iter()
        .find(|k| k.reader.eq_ignore_ascii_case(&r) && k.purpose == p)
        .and_then(|k| k.keys)
        .unwrap_or_default();
    to_jstring(env, found)
}

/// Write down a reader's keys identity, so a withdrawal can find them later.
///
/// **CALLED WHERE THE FACT ARRIVES.** A scan reads both halves of a v3 invite
/// and grants on both vaults; this records which keys member the second grant
/// created, without which `keysRevokeReader` has nobody to name. A handover
/// records the same thing at the moment it proves it — see
/// `Vault::accept_handover`.
///
/// 0 whether or not it matched: a reader this subject does not know is not an
/// error here, it is a caller doing this before granting.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_vaultNoteReaderKeys<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    vault_path: JString<'a>,
    reader_pub: JString<'a>,
    purpose: JString<'a>,
    keys_identity: JString<'a>,
) -> jlong {
    let (Ok(v), Ok(r), Ok(p), Ok(k)) = (
        env.get_string(&vault_path),
        env.get_string(&reader_pub),
        env.get_string(&purpose),
        env.get_string(&keys_identity),
    ) else {
        return -1;
    };
    let (v, r, p, k) =
        (String::from(v), String::from(r), String::from(p), String::from(k));
    let Ok(vault) = Vault::open(Path::new(&v)) else { return -3 };
    match vault.remember_reader_keys(&r, &p, &k) {
        Ok(()) => 0,
        Err(_) => -4,
    }
}

/// Withdraw, immediately. Returns the segment it takes effect from.
///
/// Rotates first, so everything written after this lands in a segment the
/// reader is not wrapped for. Not "at the next day boundary" — at UTC+12 that
/// could have been most of a day.
///
/// **THIS IS HALF A WITHDRAWAL ONCE THE KEYS VAULT HOLDS THE DATA.** The caller
/// has to follow it with `keysRevokeReader`; `vaultReaderKeys` says who to
/// name. Left as two calls rather than one because the two vaults fail
/// independently and a caller that is told which half worked can say so.
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
// SHADOW MODE SHADOWS THE VAULT THAT IS GOING TO SHIP, AND ONCE IT DID NOT.
// It sealed into `diaswarm-spaces`, which D26 decided against: that vault
// cannot express a follower who reads only the last day, because its
// application messages chain to their space's previous tips. Shadowing it was
// measuring the thing that is not going to happen. That crate and its JNI
// surface are now deleted outright.
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
/// the wrong `close` — and `SwarmNative`'s own comment says why that matters:
/// "two native handle types reachable from Kotlin is a crash waiting for
/// whoever passes the wrong one". It happened once: the close in
/// `sealPending`'s `finally` was left as `spacesClose`, which would have freed
/// this struct as a `SpacesVault` on a phone driving an insulin pump. Deleting
/// that family removed the particular wrong answer and not the hazard — the
/// tag is what makes the next one a refusal instead of undefined behaviour.
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
    /// The pool's replicator, so a published operation can be *pushed* and not
    /// merely filed.
    ///
    /// **WITHOUT THIS THE VAULT IS WRITE-TO-DISK-ONLY.** `wire::publish` puts an
    /// operation in the store, and the store is not on the network: it reaches
    /// a follower only when the next catch-up sync happens to run. Live mode —
    /// the thing that makes a follower current in seconds instead of minutes —
    /// needs the operation handed to `SyncHandle::publish`, and the handle
    /// belongs to the replicator, which belongs to the pool. A clone, because
    /// every part of a replicator that matters is already shared.
    ///
    /// `None` when the vault was opened without a pool, which is what the tests
    /// and a phone that has not joined look like.
    push: Option<diaswarm_net::replicate::KeysReplicator>,
}

/// Hand a just-published operation to live mode, and say how many peers' topics
/// it went out on.
///
/// Failure is deliberately not propagated: the operation is already in the
/// store, so the worst case is that it arrives at the next catch-up instead of
/// now. A publish that succeeded must not be reported as failed because gossip
/// was unavailable.
fn push_live(
    push: Option<&diaswarm_net::replicate::KeysReplicator>,
    signing: &p2panda_core::SigningKey,
    operation: diaswarm_keys::wire::KeysOperation,
) -> usize {
    let Some(rep) = push else { return 0 };
    rep.broadcast(&signing.verifying_key().to_hex(), operation)
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
    let push = pooled.keys_replicator.clone();

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
        match handle.block_on(diaswarm_keys::wire::publish_control(&store, &signing, &create)) {
            Ok(op) => {
                push_live(push.as_ref(), &signing, op);
            }
            Err(_) => return 0,
        }
    }
    Box::into_raw(Box::new(KeysVault { tag: KEYS_TAG, handle, store, signing, vault, push }))
        as jlong
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

/// Rotate the group secret, so what is sealed next is under a new one.
///
/// **THIS IS WHAT MAKES A SCOPED GRANT MEAN ANYTHING.** `Vault::grant_since`
/// filters the bundle by when each secret was minted, and secrets are minted per
/// group operation. A subject that never rotates holds one secret covering
/// everything, so every `since` hands over all of it or none — "share the last
/// 90 days" is a scheduling feature before it is a UI one, and the cadence sets
/// the finest window any grant can express.
///
/// Rotate daily and windows land to the day. D26 measured the cost: a year of
/// daily rotation is a 366-secret bundle and a 0.76 ms welcome, so there is no
/// performance argument for doing it less often.
///
/// ⚠️ **IT IS NOT FREE ON THE WIRE.** Each rotation is a group operation and a
/// control message every reader must receive, so this belongs on a schedule the
/// caller owns rather than on every pass.
///
/// Returns the number of secrets held after rotating, or a negative code.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_keysRotate(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jlong {
    let Some(v) = keys_vault(handle) else { return -1 };
    match v.vault.rotate() {
        Ok(_) => v.vault.secrets() as jlong,
        Err(_) => -2,
    }
}

/// Seal a batch into one epoch, read it back, and say whether it survived./// Seal a batch into one epoch, read it back, and say whether it survived.
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

    let (store, handle, signing) = (v.store.clone(), v.handle.clone(), v.signing.clone());
    let push = v.push.clone();
    to_jstring(
        env,
        seal_checked(&mut v.vault, &store, &handle, &signing, push.as_ref(), epoch, &body),
    )
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
        // **THE REASON COMES BACK, EVEN THOUGH THE CALLER IS A CHART.** This
        // returned an empty string to look like `netGlucose`, which threw away
        // the one thing that distinguishes "not granted yet" from "nothing has
        // replicated" from "the tag does not match" — three causes with three
        // different fixes, all presenting as an empty graph. The caller treats
        // anything starting `error` as no readings and says so in the log.
        return to_jstring(env, out);
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

    // **THE COUNTS COME BACK TOO, AS A LEADING `#` LINE.** `follow_read` knows
    // how many segments it opened and how many it could not; discarding that
    // left "4 readings" indistinguishable from "one segment out of twenty
    // opened". The caller logs this line and skips it.
    // `body.0` is `follow_read`'s "<opened> <unreadable>" counts.
    let mut out = format!("# opened {} rows {}\n", body.0, rows.len());
    out.push_str(
        &rows
            .iter()
            .map(|(t, mgdl, trend, src)| format!("{t}\t{mgdl}\t{trend}\t{src}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    to_jstring(env, out)
}

/// Offer our keys identity to a subject we already follow, and prove it is ours.
///
/// **THE READER'S HALF OF D27 (see `vaultAcceptHandovers` for the other).**
/// Returns 1 if the subject took it, 0 if it did not — which includes a subject
/// too old to understand the request, and is not an error: the reader goes on
/// reading the vault it already reads.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_netHandOver<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
    store_path: JString<'a>,
    identity_path: JString<'a>,
    subject_hex: JString<'a>,
    keys_identity: JString<'a>,
) -> jlong {
    let (Ok(store), Ok(id_s), Ok(subject), Ok(keys)) = (
        env.get_string(&store_path),
        env.get_string(&identity_path),
        env.get_string(&subject_hex),
        env.get_string(&keys_identity),
    ) else {
        return -1;
    };
    let store = PathBuf::from(String::from(store));
    let (subject, keys) = (String::from(subject), String::from(keys));
    let Some(mine) = load_or_create_identity(&PathBuf::from(String::from(id_s))) else {
        return -2;
    };

    // **THROUGH THE POOL'S ENDPOINT WHEN THERE IS ONE**, for the reason
    // `netRefresh` gives: a follower that dials only the address it scanned is
    // exactly as available as the subject's phone.
    if handle != 0 {
        let pooled = unsafe { &*(handle as *const Pooled) };
        return match pooled
            .runtime
            .block_on(pooled.swarm.hand_over(&store, &subject, &mine, &keys))
        {
            Ok(true) => 1,
            Ok(false) => 0,
            Err(_) => -3,
        };
    }

    let Ok(runtime) = tokio::runtime::Runtime::new() else { return -2 };
    match runtime.block_on(diaswarm_net::peer::hand_over_to(&store, &subject, &mine, &keys, None)) {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(_) => -3,
    }
}

/// Take the handover queue, keep what verifies, and say what may be granted.
///
/// **THE QUEUE IS WRITTEN BY STRANGERS AND READ HERE, WHERE THE SECRET IS.**
/// `Request::Handover`'s network handler records claims without believing any
/// of them — it answers anyone and holds no key. This is the other half: the
/// subject's own encryption secret recomputes the proof, and only a claim that
/// matches a reader already in the private book survives.
///
/// Returns `keys-identity<TAB>purpose` per verified claim, for the caller to
/// grant on the keys vault. Nothing is granted here: that needs the keys vault
/// and its store, and doing it in two steps keeps the check independent of what
/// is done with the answer.
///
/// **A CLAIM THAT DOES NOT VERIFY IS DROPPED, NOT REPORTED.** Saying which ones
/// failed, and why, would answer "is this tag one you have granted?" for
/// anybody who cared to ask — the membership question D13 exists to keep
/// private.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_vaultAcceptHandovers<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    store_path: JString<'a>,
    vault_path: JString<'a>,
    identity_path: JString<'a>,
) -> JString<'a> {
    let (Ok(store), Ok(vault_p), Ok(id_s)) = (
        env.get_string(&store_path),
        env.get_string(&vault_path),
        env.get_string(&identity_path),
    ) else {
        return to_jstring(env, String::new());
    };
    let store = PathBuf::from(String::from(store));
    let vault_p = PathBuf::from(String::from(vault_p));
    let Some(subject) = load_or_create_identity(&PathBuf::from(String::from(id_s))) else {
        return to_jstring(env, String::new());
    };
    let Ok(vault) = Vault::open(&vault_p) else {
        return to_jstring(env, String::new());
    };

    let claims = diaswarm_net::peer::take_handovers(&store).unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for claim in claims {
        let Ok(proof) = diaswarm_core::vault::unhex(&claim.proof) else { continue };
        // The purpose comes from the book, not from the claim: a claimant must
        // not be able to choose which key tree they are handed over into.
        let Ok(readers) = vault.readers() else { continue };
        let Some(known) = readers.into_iter().find(|k| k.tag == claim.tag) else { continue };
        if vault
            .accept_handover(&subject, &claim.tag, &claim.keys, &proof)
            .ok()
            .flatten()
            .is_some()
        {
            out.push(format!("{}\t{}", claim.keys, known.purpose));
        }
    }
    to_jstring(env, out.join("\n"))
}

/// One treatment record as a row, or `None` if it is not a treatment.
///
/// **ONE PLACE DECIDES THE SHAPE, BECAUSE THERE ARE TWO READERS NOW.** The core
/// vault and the keys vault both produce these, the follower parses them in one
/// function, and a difference between the two would show up as a chart that
/// changes when a subject migrates. Today has already produced four bugs of
/// exactly that kind — two halves of one contract drifting because nothing held
/// them together.
fn treatment_line(r: &Record) -> Option<String> {
    let num = |r: &Record, k: &str| r.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let txt = |r: &Record, k: &str| {
        r.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string()
    };
    Some(match r.kind() {
        "bolus" => format!("bolus\t{}\t{}\t0\t{}", r.t(), num(r, "u"), txt(r, "type")),
        "carb" => format!("carb\t{}\t{}\t{}\t", r.t(), num(r, "g"), num(r, "dur")),
        "tbr" => {
            // `abs` decides what `rate` MEANS — U/h or a percentage of basal.
            // Handing the number over without it would put a "150" on a chart
            // that could be 150% or 150 U/h.
            let abs = r.get("abs").and_then(|v| v.as_bool()).unwrap_or(false);
            format!(
                "tbr\t{}\t{}\t{}\t{}",
                r.t(),
                num(r, "rate"),
                num(r, "dur"),
                if abs { "abs" } else { "" }
            )
        }
        "extbolus" => format!("extbolus\t{}\t{}\t{}\t", r.t(), num(r, "u"), num(r, "dur")),
        _ => return None,
    })
}

/// A followed subject's newest profile record out of the keys vault.
///
/// **THE THIRD READER, AND THE ONE THAT FAILS WORST.** Glucose draws a line and
/// treatments draw marks; if either is missing you can see that it is missing.
/// The profile is arithmetic other numbers depend on: a percentage TBR carries
/// `rate` and no units, and resolving it needs the basal the subject was
/// scheduling at that moment. Without a profile `scheduledBasal` answers 0.0,
/// every percentage temp basal resolves to zero, and the chart draws a
/// confident flat line that is wrong — rather than nothing, which would at
/// least look wrong.
///
/// The target band comes from here too, and that one merely disappears.
///
/// Newest wins, as in `netProfile`: a profile record is a statement of what was
/// scheduled from then on, and older ones are history.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_keysProfile<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
    joined_dir: JString<'a>,
    subject_keys: JString<'a>,
    purpose: JString<'a>,
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

    // EVERY EPOCH, unlike the two readers above. A profile is published when it
    // changes, so the newest one may be weeks old — a tail window would answer
    // "no profile" for somebody whose settings are simply stable, which is the
    // best-run loop there is.
    let out = follow_read(
        &own_dir,
        Path::new(&joined),
        &v.store,
        &v.handle,
        &v.signing,
        &keys,
        &purpose,
        0,
        i64::MIN,
        offset,
    );
    let Some(body) = out.strip_prefix("ok ").and_then(|rest| rest.split_once('\n')) else {
        return to_jstring(env, String::new());
    };
    match newest_profile(body.1) {
        Some(json) => to_jstring(env, json),
        None => to_jstring(env, String::new()),
    }
}

/// The latest record of one kind in an ndjson body, as canonical JSON.
///
/// Latest by the record's own `t`, never by position: segments are opened per
/// epoch and concatenated, so arrival order says nothing about which record was
/// the one in force.
///
/// Used for the two kinds that are *statements rather than events* — a profile
/// and a temporary target. Both are replaced by the next one rather than
/// accumulating, and for both the cancellation is itself a record: AAPS writes
/// a temporary target of zero duration to call one off, so "newest wins"
/// handles cancelling without a special case.
fn newest_of_kind(body: &str, kind: &str) -> Option<String> {
    body.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| Record::from_json(l).ok())
        .filter(|r| r.kind() == kind)
        .max_by_key(|r| r.t())
        .map(|r| r.to_canonical_json())
}

fn newest_profile(body: &str) -> Option<String> {
    newest_of_kind(body, "profile")
}

/// Who in the pool has announced holding this subject, one node id per line.
///
/// **THE POOL'S WHOLE POINT, AND UNREADABLE UNTIL NOW.** D15 says any holder
/// serves identical bytes, so a follower whose subject is asleep should be able
/// to read from somebody else — and whether anybody else is there has never
/// been visible from a phone. When a follower sat in a pool of one all morning,
/// this is the call that would have said so in a line.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_swarmHoldersHeard<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
    subject_hex: JString<'a>,
) -> JString<'a> {
    if handle == 0 {
        return to_jstring(env, String::new());
    }
    let Ok(subject) = env.get_string(&subject_hex) else { return to_jstring(env, String::new()) };
    let subject = String::from(subject).to_ascii_lowercase();
    let pooled = unsafe { &*(handle as *const Pooled) };
    to_jstring(env, pooled.swarm.holders_heard(&subject).join("\n"))
}

/// Who in the pool has announced holding this subject's **keys** logs.
///
/// Takes the encoded keys identity a follow records — the same string
/// `keysCarryAll` decodes — rather than a bare signer, because that is what
/// the caller has and decoding it here keeps one implementation of the format.
///
/// **THE QUESTION THE KEYS VAULT COULD NOT ANSWER UNTIL NOW.** `swarmHoldersHeard`
/// reports the core vault's pool, and until 2026-09-14 the keys vault had no
/// pool at all: `keysCarryAll` carried this phone's own subject and its
/// follows, and nothing announced or adopted a stranger's. So the honest answer
/// here was always "none", and nothing on a phone said so. Now that strangers
/// carry each other's keys logs, this is how a person finds out whether the
/// redundancy is real on the pool they are actually in — as opposed to in a
/// test with two peers on one laptop.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_keysHoldersHeard<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
    keys_identity: JString<'a>,
) -> JString<'a> {
    if handle == 0 {
        return to_jstring(env, String::new());
    }
    let Ok(encoded) = env.get_string(&keys_identity) else { return to_jstring(env, String::new()) };
    let Ok(id) = diaswarm_keys::decode_identity(&String::from(encoded)) else {
        return to_jstring(env, String::new());
    };
    let pooled = unsafe { &*(handle as *const Pooled) };
    to_jstring(env, pooled.swarm.keys_holders_heard(&id.signer.to_hex()).join("\n"))
}

/// A counts line, then the last `limit` sync events, newest last, one per line.
///
/// The first line is always `counts stored=N pushed=yes|NO`: how many
/// operations this peer has stored, and whether *any* of them were pushed to it
/// in live mode rather than fetched by a catch-up sync. It is first because
/// that is the distinction between a working transport and a working poll, and
/// it is separate from the events because it is cumulative and they are a tail.
///
/// **A YES/NO, BECAUSE THE NUMBER BEHIND IT IS NOT A COUNT.** It comes from
/// `Metrics::received_live_operations` summed over sessions, and that counter
/// does not advance once per operation — `n=2` per event on one build, `n=1` on
/// another, so the multiplier is not even constant. Printed as a number beside
/// `stored` it produced `received=20401 live=21622` on a phone: more pushed
/// arrivals than arrivals. The raw value is still shown, in a parenthesis that
/// says what it is, because it is p2panda's own and hiding it would be the
/// fourth time this diagnostic was made more clever than it can support.
///
/// **Why it survives at all, given five wrong versions:** it is the only
/// *cumulative* answer. The per-arrival `live op from <peer>` lines below are
/// strictly more informative — they name the sender — but they live in a capped
/// log that rolls, so they speak for the last few minutes. This line still says
/// "a push has landed at some point since this app started" when somebody looks
/// hours later, which is when people actually look.
///
/// **THE TWO NUMBERS ARE IN DIFFERENT UNITS AND MUST NOT BE COMPARED**, which
/// is the fifth way this counter has misled. `stored` is one per operation
/// written. `live_raw` is `Metrics::received_live_operations` summed across
/// sessions, and that counter does not advance once per operation — observed at
/// `n=2` per event on one build and `n=1` on another, so the multiplier is not
/// even constant. On a phone it duly printed `received=20401 live=21622`: more
/// pushed arrivals than arrivals, which is the shape versions 2 and 3 were
/// rejected for, arrived at this time by honest reporting of a number that
/// simply is not the same quantity.
///
/// Renaming is all that is done here, deliberately. Four attempts to *derive*
/// the right number were each wrong, and dividing by a multiplier nobody has
/// measured upstream would be a fifth. What the label now says is exactly what
/// is known: `live_raw > 0` means pushes are arriving, its magnitude means
/// nothing, and the per-arrival `live op from <peer>` lines below are what to
/// count if a count is wanted.
///
///
/// **THE REPLICATOR HAS KEPT THESE ALL ALONG AND NOTHING COULD READ THEM.** Its
/// own comment says why they exist — "nothing replicated" has several very
/// different causes — and then no JNI ever exposed them, so every diagnosis of
/// replication on a phone has been guesswork from the outside. Three different
/// explanations of one stall were offered in an hour, each from about four
/// samples of a symptom, because the mechanism was unreadable.
///
/// This is a diagnostic, not a feature: it answers when a sync session happened
/// and what came of it, which is the difference between fixing a cause and
/// tuning a threshold.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_keysSyncEvents<'a>(
    env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
    limit: jlong,
) -> JString<'a> {
    if handle == 0 {
        return to_jstring(env, String::new());
    }
    let pooled = unsafe { &*(handle as *const Pooled) };
    let Some(replicator) = pooled.keys_replicator.as_ref() else {
        return to_jstring(env, "error no-replicator".to_string());
    };
    let all = replicator.events();
    let n = limit.max(1) as usize;
    let tail = if all.len() > n { &all[all.len() - n..] } else { &all[..] };
    // **THE TWO COUNTS, SEPARATELY, BECAUSE THE TOTAL HID THE BUG.** Operations
    // kept arriving and the phone looked healthy, while every one of them came
    // from a catch-up sync and live mode delivered nothing for the life of the
    // app. One number could not have shown that and did not.
    //
    // And they are NAMED apart because putting them side by side under one word
    // invited the comparison the doc comment above explains is meaningless.
    let live = replicator.live_received();
    let mut out = format!(
        "counts stored={} pushed={}{}",
        replicator.received(),
        if live > 0 { "yes" } else { "NO — everything arrived by catch-up" },
        if live > 0 { format!(" (p2panda's raw counter {live}, not a count of anything)") }
        else { String::new() },
    );
    for line in tail {
        out.push('\n');
        out.push_str(line);
    }
    to_jstring(env, out)
}

/// Re-subscribe every keys topic, now.
///
/// **THE TARGETED REMEDY FOR A ONE-SHOT SUBSCRIPTION.** `stream` catches up once
/// and then waits for gossip; when that link dies the follower goes quiet and
/// nothing re-establishes it, while `keysCarryAll` keeps returning the cached
/// count. Restarting the whole endpoint also fixes it and is a sledgehammer —
/// it drops every connection this phone has, including the ones that are
/// working.
///
/// **THE CALLER DECIDES WHEN.** This used to take a quiet period and compare it
/// against the last operation received — which on a phone meant it never fired,
/// because catch-up syncs kept delivering operations while the newest *record*
/// aged past a thousand seconds. The app knows how stale the data is; this does
/// not and cannot.
///
/// Returns the number of topics re-streamed. -3 if the pool has no keys
/// replicator, matching every other keys call.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_keysRestream<'a>(
    _env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
) -> jlong {
    if handle == 0 {
        return -1;
    }
    let pooled = unsafe { &*(handle as *const Pooled) };
    let Some(replicator) = pooled.keys_replicator.as_ref() else { return -3 };
    match pooled.runtime.block_on(replicator.restream()) {
        Ok(n) => n as jlong,
        Err(_) => -4,
    }
}

/// Check a followed subject's CORE grant log holds together.
///
/// **THE OTHER VAULT'S HALF OF THE SAME PROPERTY.** `keysVerifyControl` covers
/// the keys group's control log; this covers `grants.ndjson`, which is the
/// signed record §11 offers in place of a read log and the one that is live
/// today. `verify_own_chain` was written with the signing key that made it
/// checkable by anyone, has tests, and — like its keys twin — was reachable
/// from neither app.
///
/// Runs against the replica this follower holds, which is the copy worth
/// checking: a subject verifying their own log catches nobody.
///
/// `ok` when every entry follows its predecessor, `broken <seq>` at the first
/// that does not, `error <why>` otherwise. Never empty: a check that answers
/// nothing reads exactly like a check that passed.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_vaultVerifyChain<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    store_path: JString<'a>,
    subject: JString<'a>,
) -> JString<'a> {
    let (Ok(store), Ok(subj)) = (env.get_string(&store_path), env.get_string(&subject)) else {
        return to_jstring(env, "error bad-argument".to_string());
    };
    let dir = PathBuf::from(String::from(store)).join(String::from(subj));
    let Ok(vault) = Vault::open(&dir) else { return to_jstring(env, "error no-vault".to_string()) };
    match vault.verify_own_chain() {
        Ok(None) => to_jstring(env, "ok".to_string()),
        Ok(Some(seq)) => to_jstring(env, format!("broken {seq}")),
        // A vault written before the signing key was published cannot be
        // checked by anyone. That is a real answer, and it is NOT "ok" — the
        // whole point of publishing the key was that "cannot check" stopped
        // being a quiet pass.
        Err(e) => to_jstring(env, format!("error {e:?}")),
    }
}

/// Check a subject's control log holds together, and say what it found.
///
/// **D13's TAMPER-EVIDENCE WAS WRITTEN, TESTED, AND NEVER RUN.** The grant log
/// replicates so that truncating or altering it is *detectable* — that is the
/// whole of what feasibility.md §11 offers in place of a read log. The code to
/// detect it has existed since the auth layer landed and nothing on either
/// phone has ever called it. A property that only holds when somebody runs a
/// test is not a property of the system.
///
/// Runs on the replicated copy this phone holds, which is the copy that
/// matters: the subject cannot meaningfully catch themselves, and a follower
/// checking the log it was actually served is the only party whose check means
/// anything.
///
/// Returns `ok <entries> <head>`, `broken <seq>`, or `error <why>`. Never
/// empty and never throws: a check that answers nothing is indistinguishable
/// from a check that passed, which is how the first version of this ran on a
/// phone for ten minutes doing nothing at all.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_keysVerifyControl<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
    subject_hex: JString<'a>,
) -> JString<'a> {
    let Ok(subject) = env.get_string(&subject_hex) else {
        return to_jstring(env, "error bad-argument".to_string());
    };
    let subject = String::from(subject);
    // **THE IDENTITY, NOT A BARE KEY — because that is what a follow records.**
    // The first version of this took 64 hex characters and every caller had a
    // `KeysIdentity`, so it answered empty on every pass and the app's own
    // "nothing replicated yet" branch swallowed it. Both forms are accepted for
    // the same reason `keysGrant` accepts both: whichever a caller has should
    // work.
    let author = match diaswarm_keys::decode_identity(&subject) {
        Ok(id) => id.signer,
        Err(_) => match verifying_key(&subject.to_ascii_lowercase()) {
            Some(k) => k,
            None => return to_jstring(env, "error not-an-identity".to_string()),
        },
    };
    let Some(v) = keys_vault(handle) else { return to_jstring(env, "error no-vault".to_string()) };

    let chain = v.handle.block_on(diaswarm_keys::auth::verify_control_chain(&v.store, &author));
    match chain {
        Ok(c) => {
            let out = match c.broken_at {
                Some(seq) => format!("broken {seq}"),
                None => format!(
                    "ok {} {}",
                    c.len(),
                    c.head().map(|h| h.to_string()).unwrap_or_else(|| "-".into())
                ),
            };
            to_jstring(env, out)
        }
        Err(e) => to_jstring(env, format!("error {e}")),
    }
}

/// A followed subject's newest temporary target out of the keys vault.
///
/// The keys-vault half of `netTempTarget`; see that for why it exists. Reads
/// every epoch rather than a tail, for the same reason the profile does: a
/// temporary target set yesterday and still running would be invisible to a
/// window that only looks at today.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_keysTempTarget<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
    joined_dir: JString<'a>,
    subject_keys: JString<'a>,
    purpose: JString<'a>,
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
    let out = follow_read(
        &own_dir,
        Path::new(&joined),
        &v.store,
        &v.handle,
        &v.signing,
        &keys,
        &purpose,
        0,
        i64::MIN,
        offset,
    );
    let Some(body) = out.strip_prefix("ok ").and_then(|rest| rest.split_once('\n')) else {
        return to_jstring(env, String::new());
    };
    match newest_of_kind(body.1, "target") {
        Some(json) => to_jstring(env, json),
        None => to_jstring(env, String::new()),
    }
}

/// A followed subject's treatments out of the keys vault, in
/// `netTreatments`' shape.
///
/// **THE HALF THAT WOULD HAVE GONE MISSING AT A CUTOVER.** `keysGlucose` was
/// written first and on its own it is enough for the graph line — so a keys
/// read looks healthy while bolus, carb and basal quietly vanish from it. The
/// emitter has always drained all four; only the reader was incomplete.
///
/// Same rows as `netTreatments` so the follower's parsing and its chart are
/// untouched, and the same overlap rule: segments legitimately repeat, and a
/// bolus drawn twice is a bolus that looks like two.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_keysTreatments<'a>(
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

    let tail = if since_ms > 0 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(since_ms);
        (((now.saturating_sub(since_ms)) / 86_400_000) + 2).max(2) as u64
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
        return to_jstring(env, String::new());
    };

    let mut rows: Vec<(i64, String)> = body
        .1
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| Record::from_json(l).ok())
        .filter(|r| r.t() > since_ms)
        .filter_map(|r| treatment_line(&r).map(|line| (r.t(), line)))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    rows.dedup_by(|a, b| a.1 == b.1);
    if limit > 0 {
        rows.truncate(limit as usize);
    }
    to_jstring(env, rows.iter().map(|(_, l)| l.clone()).collect::<Vec<_>>().join("\n"))
}

/// Carry every keys log this phone should hold: its own, each it follows, and
/// each stranger's that falls in this phone's share of the pool.
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
/// **AND STRANGERS', WHICH IS THE HALF THAT WAS MISSING.** Until 2026-09-14
/// this carried our own subject and our follows and stopped. Pool adoption
/// announced and fetched *core*-vault subjects only, so
/// [D15](../../docs/decisions.md) — any holder serves identical bytes, so a
/// subject whose phone is asleep stays readable — was true of the vault being
/// replaced and false of the vault replacing it. After a cutover a follower
/// could reach a subject at exactly one address: the subject's own phone. That
/// is not a swarm, and it is the property the whole design is for.
///
/// The two halves are one change on purpose. Announcing what we hold without
/// strangers holding anything would publish the follower set — a peer saying
/// "I hold keys-subject Y" would mean "I am watching Y's glucose", which is
/// the leak [D18](../../docs/decisions.md) was retired for. Carrying
/// strangers' subjects is what makes that sentence ambiguous.
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
    max_adopt: jint,
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

    // THE DECISION LIVES IN `diaswarm-net`, NOT HERE. Working out what a peer
    // should hold is a property of being a peer; a desktop daemon needs exactly
    // the same rule ([D29](../../docs/decisions.md)), and two copies of it would
    // be two copies of what a peer owes the pool. What stays on this side is
    // what is genuinely Android's: raw handles, JStrings, and negative codes
    // instead of exceptions.
    let own = p2panda_core::SigningKey::from_bytes(&identity.signing.to_bytes())
        .verifying_key()
        .to_hex();
    match pooled.runtime.block_on(diaswarm_net::share::carry_share(
        &pooled.swarm,
        replicator,
        &store,
        &own,
        max_adopt.max(0) as usize,
        // A phone is told whom to carry by pairing, not by a flag.
        &[],
    )) {
        Ok(share) => share.carrying as jlong,
        Err(_) => -4,
    }
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
    grant_and_publish(env, v, bundle, &purpose, false)
}

/// Withdraw a reader from the keys vault, naming them by their identity rather
/// than by a tag nobody kept.
///
/// **THE OTHER HALF OF A WITHDRAWAL.** A reader is two members: one in the core
/// vault, wrapped per segment, and one in the keys group. Granting does both —
/// a scan that did only one was fixed as soon as it was seen. Revoking did only
/// the core one, so after the cutover, withdrawing would have removed somebody
/// from the vault that no longer holds the data and left them reading the one
/// that does. A safety control that silently does nothing is worse than one
/// that is missing.
///
/// It takes the identity, not the tag, because D13's tag is *derived*: the same
/// pair and purpose always name the same member, so there is no book to keep
/// and none to fall out of step. `keysGrant` computes it one way in; this
/// computes the identical thing on the way out.
///
/// 0 on success, and 0 again when they were not a member — withdrawing from
/// somebody who is already out is the outcome the caller asked for.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_keysRevokeReader<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    handle: jlong,
    reader_bundle: JString<'a>,
    purpose: JString<'a>,
) -> jlong {
    let (Ok(bundle), Ok(purpose)) = (env.get_string(&reader_bundle), env.get_string(&purpose))
    else {
        return -1;
    };
    let (bundle, purpose) = (String::from(bundle), String::from(purpose));
    let bundle = match diaswarm_keys::decode_identity(&bundle) {
        Ok(id) => id.bundle,
        Err(_) => match diaswarm_keys::decode_bundle(&bundle) {
            Ok(b) => b,
            Err(_) => return -2,
        },
    };
    let Some(v) = keys_vault(handle) else { return -3 };
    let tag = match v.vault.tag_of(bundle, &purpose) {
        Ok(t) => t,
        Err(_) => return -4,
    };
    match v.vault.revoke(tag) {
        Ok(message) => {
            match v.handle.block_on(diaswarm_keys::wire::publish_control(
                &v.store,
                &v.signing,
                &message,
            )) {
                Ok(op) => {
                    push_live(v.push.as_ref(), &v.signing, op);
                }
                Err(_) => return -5,
            }
            0
        }
        // NOT A MEMBER IS THE ANSWER THE CALLER WANTED. They asked for this
        // reader to be unable to read, and they cannot.
        Err(diaswarm_keys::Error::Group(_)) => 0,
        Err(_) => -6,
    }
}

/// Grant from a D27 handover: the same grant, minus the authority to reverse a
/// revoke.
///
/// **A SEPARATE ENTRY POINT, NOT A FLAG WITH A DEFAULT.** The two callers are a
/// person scanning an invite and a message arriving over the network, and only
/// the first may let a revoked reader back in. A boolean argument would put
/// that distinction somewhere it can be got wrong by omission; two names put it
/// at the call site where it is read.
///
/// Returns the same 64 hex characters, or `error revoked` — which is a refusal
/// working, not a failure.
#[no_mangle]
pub extern "system" fn Java_nz_diaswarm_jni_SwarmNative_keysGrantUnattended<'a>(
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
    let bundle = match diaswarm_keys::decode_identity(&bundle) {
        Ok(id) => id.bundle,
        Err(_) => match diaswarm_keys::decode_bundle(&bundle) {
            Ok(b) => b,
            Err(e) => return to_jstring(env, format!("error bundle {e}")),
        },
    };
    grant_and_publish(env, v, bundle, &purpose, true)
}

/// Add the reader, publish the welcome if there is one, and answer with the tag.
fn grant_and_publish<'a>(
    env: JNIEnv<'a>,
    v: &mut KeysVault,
    bundle: diaswarm_keys::LongTermKeyBundle,
    purpose: &str,
    unattended: bool,
) -> JString<'a> {
    let granted = if unattended {
        v.vault.grant_unattended(bundle, purpose)
    } else {
        v.vault.grant(bundle, purpose)
    };
    let tag = match granted {
        Ok((welcome, tag)) => {
            match v.handle.block_on(diaswarm_keys::wire::publish_control(
                &v.store,
                &v.signing,
                &welcome,
            )) {
                Ok(op) => {
                    push_live(v.push.as_ref(), &v.signing, op);
                }
                Err(e) => return to_jstring(env, format!("error publish {e}")),
            }
            tag
        }
        // **ALREADY IN IS SUCCESS, AND SAYING SO IS THE POINT.** The caller
        // asked for this reader to be able to read, and they can. Publishing a
        // second welcome would be the bug: see `Error::AlreadyGranted`. The tag
        // is the same one the first grant returned, so the caller's bookkeeping
        // is unchanged and a retry is indistinguishable from a first attempt —
        // which is what makes the follower's retries free.
        Err(diaswarm_keys::Error::AlreadyGranted(tag)) => tag,
        // **A REFUSAL WORKING, SAID OUT LOUD.** Only the unattended door can
        // reach this. It is not an error the caller should retry away: it means
        // somebody who was revoked tried to come back through a handover.
        Err(diaswarm_keys::Error::Revoked(_)) => {
            return to_jstring(env, "error revoked".to_string());
        }
        Err(e) => return to_jstring(env, format!("error grant {e}")),
    };
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
    match v.handle.block_on(diaswarm_keys::wire::publish_control(&v.store, &v.signing, &message)) {
        Ok(op) => {
            push_live(v.push.as_ref(), &v.signing, op);
        }
        Err(_) => return -5,
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
pub fn seal_checked(
    vault: &mut diaswarm_keys::Vault,
    store: &diaswarm_keys::SqliteStore,
    runtime: &tokio::runtime::Handle,
    signing: &p2panda_core::SigningKey,
    push: Option<&diaswarm_net::replicate::KeysReplicator>,
    epoch: i64,
    body: &str,
) -> String {
    let given: Vec<Record> = body
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| Record::from_json(l).ok())
        .map(Record::normalise)
        .collect();

    if let Err(e) = vault.seal(epoch, &given) {
        return format!("error seal {e}");
    }

    // **THE DELTA IS WHAT GETS PUBLISHED, NOT THE MERGED DAY.** `seal` returns
    // the whole day so a follower could be handed 23 MB of log for 160 kB of
    // data — 144× — and 700× if the cadence were raised to make the follower
    // less stale. A delta is 1× at any cadence, which is what lets this publish
    // on every pass instead of every five minutes.
    let segment = match vault.seal_delta(epoch, &given) {
        Ok(s) => s,
        Err(e) => return format!("error seal-delta {e}"),
    };

    // **AND PUBLISH IT, OR NOBODY CAN EVER FETCH IT.** Sealing writes a file;
    // a follower reads operations. Without this the segments existed only on
    // the subject's own filesystem, so a reader could be granted, replicate the
    // control log, find its welcome and join — all of which worked — and then
    // have nothing to open. The failure looked like a broken reader and was a
    // missing writer.
    let pushed = match runtime.block_on(diaswarm_keys::wire::publish(store, signing, &segment)) {
        // **AND PUSH IT, OR IT ARRIVES WHEN THE NEXT SYNC HAPPENS TO RUN.**
        // Storing makes it fetchable; only this makes it delivered. See
        // `KeysReplicator::broadcast` for what was measured before this line
        // existed.
        Ok(op) => push_live(push, signing, op),
        Err(e) => return format!("error publish {e}"),
    };

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

    // **`pushed` IS IN THE LINE BECAUSE IT IS THE ONLY WAY TO SEE LIVE MODE
    // WORKING FROM THE SENDING SIDE.** A `0` here with a follower carrying the
    // subject means the push half is broken again, which is exactly the state
    // this project shipped in unnoticed for its whole life. It is a count of
    // topics the operation went out on, not of peers that got it.
    format!(
        "ok epoch={epoch} given={} held={} missing={missing} lost={} pushed={pushed}",
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
    // **ONE EPOCH IS MANY OPERATIONS, AND ONLY THE LAST ONE MATTERS.**
    //
    // A subject publishes on every flush — every five minutes — and each flush
    // re-seals the whole accumulated day, so the operations for one epoch are a
    // sequence of supersets and the newest contains all of them. That breaks
    // the invariant `segments_tail` was written under, which was "the last N
    // entries are the last N days": at a five-minute cadence the last two
    // entries are ten minutes of today.
    //
    // So the tail is asked for in *operations* rather than days — 288 flushes
    // to a day, plus slack for a re-drain — and then reduced to one segment per
    // epoch, keeping the last, which is the complete one.
    //
    // The cost is fetching supersets that are then discarded. It is the price
    // of a follower seeing today as it happens rather than after midnight, and
    // it is the thing to revisit first if replication gets expensive.
    let segments: Vec<diaswarm_keys::Segment> = if tail_days > 0 {
        let entries = tail_days.saturating_mul(320).min(20_000);
        let tail = runtime
            .block_on(diaswarm_keys::wire::segments_tail(store, subject, entries))
            .map_err(|e| format!("segments {e}"))?;
        // **AND THEN THE NEWEST `tail_days` OF THEM.** Asking the log for 320
        // operations a day is how many entries to *fetch*; it says nothing
        // about how many days those entries cover, and on a quiet log they
        // cover far more. A follower asking for one day must get one day —
        // `a_tail_read_returns_the_newest_days_and_no_others` caught this
        // returning thirty.
        let run: Vec<diaswarm_keys::Segment> =
            tail.into_iter().filter(|s| s.epoch >= from_epoch).collect();
        // Keep every segment belonging to the newest `tail_days` epochs.
        let epochs = epochs_of(&run);
        let keep: std::collections::HashSet<i64> =
            epochs.iter().rev().take(tail_days as usize).copied().collect();
        run.into_iter().filter(|s| keep.contains(&s.epoch)).collect()
    } else {
        runtime
            .block_on(diaswarm_keys::wire::segments_from(store, subject, from_epoch))
            .map_err(|e| format!("segments {e}"))?
    };

    let mut out = String::new();
    let mut opened = 0usize;
    let mut unreadable = 0usize;
    // **DEDUPED, BECAUSE A RE-DRAIN REPUBLISHES.** Deltas do not normally
    // overlap, but a subject that re-reads its whole database seals the same
    // records again, and a follower must not show a reading twice.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for segment in &segments {
        match vault.open_segment(segment) {
            Ok((records, bad)) => {
                opened += 1;
                unreadable += bad;
                for record in records {
                    let line = record.to_canonical_json();
                    if seen.insert(line.clone()) {
                        out.push_str(&line);
                        out.push('\n');
                    }
                }
            }
            // NOT AN ERROR. A segment sealed before this reader was granted, or
            // after it was revoked, is access control working.
            Err(_) => unreadable += 1,
        }
    }
    Ok((out, opened, unreadable))
}

/// The distinct epochs a run of segments covers, newest last.
///
/// **ONE EPOCH IS MANY SEGMENTS AND THEY ALL COUNT.** Each publish carries only
/// the records added since the last one, so a day is the concatenation of its
/// segments rather than the last of them. An earlier version kept only the
/// newest per epoch, which was right when each publish carried the whole day
/// and silently discards 99% of it now.
fn epochs_of(segments: &[diaswarm_keys::Segment]) -> Vec<i64> {
    let mut seen: Vec<i64> = segments.iter().map(|s| s.epoch).collect();
    seen.sort_unstable();
    seen.dedup();
    seen
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

    /// A vault, its store, a runtime and the key that signs its log.
    ///
    /// The store is here because sealing now publishes: a segment that is only
    /// a file is a segment no follower can fetch, which is what the device
    /// showed after the join already worked.
    fn vault(
        tag: &str,
    ) -> (
        diaswarm_keys::Vault,
        diaswarm_keys::SqliteStore,
        tokio::runtime::Runtime,
        p2panda_core::SigningKey,
    ) {
        let root = std::env::temp_dir().join(format!(
            "diaswarm-shadow-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let signing = p2panda_core::SigningKey::generate();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let store = rt.block_on(async {
            diaswarm_keys::SqliteStoreBuilder::memory().build().await.unwrap()
        });
        let mut v = diaswarm_keys::Vault::open(&root, 12 * 3_600_000, &signing).unwrap();
        let rng = diaswarm_keys::Rng::default();
        let (manager, _bundle) = diaswarm_keys::Vault::key_bundle(&rng).unwrap();
        v.create(manager).unwrap();
        (v, store, rt, signing)
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

    /// A DAY PUBLISHED AS MANY DELTAS READS BACK AS THE WHOLE DAY.
    ///
    /// **THE SHAPE THE WIRE ACTUALLY CARRIES.** A subject publishes only the
    /// records added since its last flush, so one epoch is a run of segments
    /// that concatenate. Keeping just the newest — which was right when each
    /// publish carried the whole re-sealed day — silently discards everything
    /// but the last few minutes.
    ///
    /// Found because a follower showed a reading four minutes old: the cadence
    /// that made it stale existed to bound the cost of publishing whole days,
    /// and deltas remove both the cost and the reason.
    #[test]
    fn a_day_published_as_deltas_reads_back_whole() {
        use diaswarm_keys::{Vault, encode_identity, wire};

        let rt = tokio::runtime::Runtime::new().unwrap();
        let store = rt.block_on(async {
            diaswarm_keys::SqliteStoreBuilder::memory().build().await.unwrap()
        });
        let rng = diaswarm_keys::Rng::default();

        let me = p2panda_core::SigningKey::generate();
        let own_dir = dir("d-own");
        let mut own = Vault::open(&own_dir, 12 * 3_600_000, &me).unwrap();
        let (own_mgr, _b) = Vault::key_bundle(&rng).unwrap();
        own.create(own_mgr).unwrap();
        let my_bundle = diaswarm_keys::encode_bundle(&own.my_bundle().unwrap()).unwrap();
        drop(own);

        let subject_key = p2panda_core::SigningKey::generate();
        let mut subject = Vault::open(dir("d-subject"), 12 * 3_600_000, &subject_key).unwrap();
        let (s_mgr, _sb) = Vault::key_bundle(&rng).unwrap();
        let create = subject.create(s_mgr).unwrap();
        rt.block_on(wire::publish_control(&store, &subject_key, &create)).unwrap();
        let subject_keys = encode_identity(&subject.identity().unwrap()).unwrap();
        let reader = diaswarm_keys::decode_bundle(&my_bundle).unwrap();
        let (welcome, _t) = subject.grant(reader, "follow").unwrap();
        rt.block_on(wire::publish_control(&store, &subject_key, &welcome)).unwrap();

        // One day, twelve flushes, five records each — the way a phone does it.
        let mut expected = 0usize;
        for flush in 0..12i64 {
            let batch: Vec<diaswarm_core::Record> = (0..5i64)
                .map(|i| {
                    diaswarm_core::Record::new(
                        33_000 * 86_400_000 + (flush * 5 + i) * 300_000,
                        "cgm",
                    )
                    .set("mgdl", Some((100.0 + i as f64).into()))
                })
                .collect();
            subject.seal(33_000, &batch).unwrap();
            let delta = subject.seal_delta(33_000, &batch).unwrap();
            rt.block_on(wire::publish(&store, &subject_key, &delta)).unwrap();
            expected += batch.len();
        }

        let author = subject_key.verifying_key();
        let out = super::follow_read(
            &own_dir,
            &dir("d-joined"),
            &store,
            rt.handle(),
            &me,
            &subject_keys,
            "follow",
            1,
            i64::MIN,
            12 * 3_600_000,
        );
        assert!(out.starts_with("ok "), "read failed: {}", &out[..out.len().min(60)]);
        let lines = out.lines().skip(1).filter(|l| !l.trim().is_empty()).count();
        assert_eq!(
            lines, expected,
            "a day published as 12 deltas read back as {lines} of {expected} records"
        );
        let _ = author;
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
        let (mut v, store, rt, key) = vault("format");
        let report = seal_checked(&mut v, &store, rt.handle(), &key, None, 20_000, &ndjson(20_000 * 86_400_000, 12));

        assert!(report.starts_with("ok "), "unexpected report: {report}");
        let fields: std::collections::HashMap<&str, &str> = report
            .split(' ')
            .filter_map(|f| f.split_once('='))
            .collect();
        // `pushed` is in this list because the plugin sums it into the line
        // that says whether live mode is sending anything. A build where it
        // silently stopped being emitted would report `pushed 0` for ever,
        // which is indistinguishable from the defect it exists to detect.
        for key in ["epoch", "given", "held", "missing", "lost", "pushed"] {
            let value = fields.get(key).unwrap_or_else(|| panic!("no {key} in {report}"));
            value.parse::<i64>().unwrap_or_else(|_| panic!("{key} is not a number in {report}"));
        }
        assert_eq!(fields["given"], "12");
        assert_eq!(fields["missing"], "0", "a fresh seal lost records: {report}");
        assert_eq!(fields["lost"], "0");
        // No pool here, so nothing to push onto — and it says 0 rather than
        // failing the seal. A vault that refused to seal because it could not
        // gossip would be a worse bug than the one this fixes.
        assert_eq!(fields["pushed"], "0", "a vault with no pool claimed a push: {report}");
    }

    /// A DAY ARRIVING IN PIECES STILL REPORTS NOTHING MISSING.
    ///
    /// The shape a phone actually produces: the plugin flushes what has
    /// accumulated since the last cadence, not the whole day. `held` grows and
    /// `missing` stays at zero — and when `seal` replaced the segment instead
    /// of appending to it, this is the test that would have said so.
    #[test]
    fn a_day_sealed_in_pieces_reports_nothing_missing() {
        let (mut v, store, rt, key) = vault("pieces");
        let base = 20_001 * 86_400_000;
        let mut held = 0i64;
        for chunk in 0..6 {
            let report = seal_checked(&mut v, &store, rt.handle(), &key, None, 20_001, &ndjson(base + chunk * 12 * 300_000, 12));
            let fields: std::collections::HashMap<&str, &str> =
                report.split(' ').filter_map(|f| f.split_once('=')).collect();
            assert_eq!(fields["missing"], "0", "flush {chunk} lost records: {report}");
            held = fields["held"].parse().unwrap();
        }
        assert_eq!(held, 72, "the day did not accumulate across flushes");
    }

    /// A DIAGNOSTIC NOTHING CAN READ IS NOT A DIAGNOSTIC.
    ///
    /// **THE MOST EXPENSIVE LESSON OF 2026-09-14.** The replicator recorded
    /// every sync event from the day it was written, with a comment saying why
    /// — "nothing replicated" has several very different causes — and no JNI
    /// ever exposed it. So a replication problem on a phone could only be
    /// reasoned about from the outside, and one afternoon produced three
    /// contradictory diagnoses from four samples each before anyone thought to
    /// look at what the code already knew. `verify_control_chain` and
    /// `swarmTick` were the same pattern in other layers.
    ///
    /// A function whose name promises to report state, and which no app can
    /// reach, is a question that cannot be asked at the only moment it matters.
    /// So they are listed, and a new one has to be either wired up or written
    /// down here with the reason it is unreachable.
    ///
    /// This is a naming heuristic and it will miss things. It is still better
    /// than the alternative, which today was noticing on the fourth guess.
    #[test]
    fn diagnostics_can_be_read_from_a_phone() {
        // Unreachable on purpose, with the reason.
        const NOT_WIRED: &[(&str, &str)] = &[
            ("verify", "the inner helper; verify_own_chain is what the JNI calls"),
            ("verify_chain", "same — reached through verify_own_chain"),
            ("verifying", "an Identity accessor, not a report"),
            ("message", "a Grant accessor, not a report"),
        ];

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut names: Vec<String> = Vec::new();
        for krate in ["../diaswarm-net/src", "../diaswarm-keys/src", "../diaswarm-core/src"] {
            let Ok(entries) = std::fs::read_dir(root.join(krate)) else { continue };
            for e in entries.filter_map(|e| e.ok()) {
                let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
                for line in text.lines() {
                    let t = line.trim();
                    let Some(rest) = t
                        .strip_prefix("pub fn ")
                        .or_else(|| t.strip_prefix("pub async fn "))
                    else {
                        continue;
                    };
                    let name: String =
                        rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                    if !name.is_empty() {
                        names.push(name);
                    }
                }
            }
        }

        // Names that promise to report something a person would want at 3am.
        const REPORTS: &[&str] = &[
            "report", "status", "events", "stats", "metrics", "health", "verify", "heard", "seen",
        ];
        let jni = std::fs::read_to_string(root.join("src/lib.rs")).expect("lib.rs");

        let mut unreachable: Vec<String> = names
            .into_iter()
            .filter(|n| REPORTS.iter().any(|r| n.contains(r)))
            .filter(|n| !NOT_WIRED.iter().any(|(d, _)| d == n))
            // BOTH CALL SHAPES. The first version matched only `.name(` and
            // reported `verify_control_chain` as unreachable when the JNI calls
            // it as a free function, `auth::verify_control_chain(...)`. A guard
            // that cries wolf gets an entry added to NOT_WIRED to shut it up,
            // which is how a guard stops guarding.
            .filter(|n| !jni.contains(&format!(".{n}(")) && !jni.contains(&format!("::{n}(")))
            .collect();
        unreachable.sort();
        unreachable.dedup();

        assert!(
            unreachable.is_empty(),
            "these report state and no app can read them: {unreachable:?}\n\
             A diagnostic nothing can reach is a question that cannot be asked at \n\
             the only moment it matters. Wire it up, or add it to NOT_WIRED with a reason."
        );
    }

    /// EVERY JNI DECLARATION IS EITHER CALLED, OR ON THE LIST OF ONES THAT ARE NOT.
    ///
    /// **THE AUDIT I DID NOT RUN, AND IT COST A NIGHT.** On 2026-09-14 a
    /// follower went blind because `swarmTick` was declared in `SwarmNative`
    /// and called from nowhere: the app joined the pool and never took a turn
    /// in it, so it could only ever reach a subject at the address it was
    /// handed. Two audits had been run that week — emitted record kinds against
    /// consumed ones, public Rust functions against called ones — and neither
    /// pointed at this seam, which is where the defect was.
    ///
    /// A bare "everything must be called" rule is no good here: several
    /// declarations are dead *by decision* — a few keys helpers were
    /// superseded, and two core helpers are exercised from Rust rather than
    /// Kotlin. So the dead ones are listed by name. The list is the point:
    /// adding a declaration and forgetting to call it fails immediately, and so
    /// does quietly dropping the last call to a live one.
    ///
    /// **THE `spaces*` FAMILY USED TO BE SEVEN OF THESE ENTRIES.** They were
    /// dead by decision for long enough to look permanent, and were shipped to
    /// two phones all that time. A list of knowingly-dead declarations is a
    /// holding pen, not a destination: when the reason is "a decision went the
    /// other way", the answer is to delete the code.
    ///
    /// If this fails, the fix is one of three things — call it, delete it, or
    /// put it on the list with a reason. Not the third by reflex.
    #[test]
    fn every_jni_declaration_is_called_or_knowingly_dead() {
        // Dead by decision, each with the reason it is still declared.
        const KNOWN_DEAD: &[(&str, &str)] = &[
            ("header", "core helper, exercised by the Rust tests"),
            ("canonicalLine", "core helper, exercised by the Rust tests"),
            ("keysSubject", "superseded by keysIdentity"),
            ("keysRevoke", "superseded by keysRevokeReader, which derives the tag"),
            ("keysFollowRead", "superseded by keysGlucose/keysTreatments/keysProfile"),
            ("keysCarry", "superseded by keysCarryAll"),
        ];

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let decls = std::fs::read_to_string(
            root.join("plugin/src/main/kotlin/nz/diaswarm/jni/SwarmNative.kt"),
        )
        .expect("SwarmNative.kt");

        // Every call site in either app, minus the declarations themselves.
        let mut callers = String::new();
        for dir in ["follower/src/main/kotlin", "plugin/src/main/kotlin"] {
            collect_kotlin(&root.join(dir), &mut callers);
        }

        let mut orphans = Vec::new();
        for line in decls.lines() {
            let Some(rest) = line.trim().strip_prefix("external fun ") else { continue };
            let name: String =
                rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if name.is_empty() || KNOWN_DEAD.iter().any(|(d, _)| *d == name) {
                continue;
            }
            // A call is `SwarmNative.name(` from outside, or bare `name(` from
            // inside the object itself — `check()` calls `specVersion()`.
            let qualified = format!("SwarmNative.{name}(");
            let bare = format!(" {name}()");
            if !callers.contains(&qualified) && !callers.contains(&bare) {
                orphans.push(name);
            }
        }

        assert!(
            orphans.is_empty(),
            "declared in SwarmNative and called by neither app: {orphans:?}\n\
             Call it, delete it, or add it to KNOWN_DEAD with the reason."
        );
    }

    /// AN APP THAT JOINS THE POOL MUST ALSO TAKE A TURN IN IT.
    ///
    /// **THE RULE THE LAST TEST COULD NOT EXPRESS, AND THE ONE THAT MATTERS.**
    /// Checking that every declaration is called *by some app* cannot catch
    /// what actually happened: AAPS called `swarmTick` all along and Ayni never
    /// did, so the union looked complete while the follower sat in a pool of
    /// one, unable to find a subject that had moved. Verified by mutation —
    /// deleting Ayni's call leaves the union check green.
    ///
    /// So this is per app, and narrow enough to be true rather than tidy. Not
    /// every app should call every function: a follower has no business
    /// granting, and the publisher does not read its own vault. What every
    /// pool member owes the pool is the same three things — join it, take a
    /// turn in it, leave it — and a member that only joins is a member that
    /// learns nothing and is never found.
    #[test]
    fn every_app_that_joins_the_pool_also_ticks_it() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for app in ["follower/src/main/kotlin", "plugin/src/main/kotlin"] {
            let mut src = String::new();
            collect_kotlin(&root.join(app), &mut src);
            if !src.contains("SwarmNative.swarmJoin(") {
                continue; // does not join, owes the pool nothing
            }
            for required in ["swarmTick", "swarmLeave"] {
                assert!(
                    src.contains(&format!("SwarmNative.{required}(")),
                    "{app} calls swarmJoin but never {required}.\n\
                     A member that joins and never ticks announces nothing, learns nobody, \n\
                     and can only reach a subject at the address it was handed — which works \n\
                     until that address changes. That is the 2026-09-14 outage."
                );
            }
        }
    }

    /// AND IT MUST CARRY A SHARE, NOT ONLY TAKE A TURN.
    ///
    /// **THE RULE THE TICK GUARD CANNOT EXPRESS, AND THE ONE THE NAME PROMISES.**
    /// Ayni passed `0` as its adoption budget to both `swarmTick` and
    /// `keysCarryAll` for a day: it joined the pool, ticked it, announced what
    /// it held, and adopted nothing for anybody. Every guard in this file was
    /// green throughout, because each of them asks whether a call happens and
    /// this defect is in an argument.
    ///
    /// It matters more than it looks. A subject is highly available because
    /// *other* phones hold it, and followers are most of the phones. If every
    /// follower carries nothing, the only peer holding a subject is the subject
    /// — which is the property D15 exists to remove, arrived at by default.
    /// `strings.xml` names the app for the opposite: "your phone carries other
    /// people's sealed records so that yours are carried when your phone is
    /// off".
    ///
    /// So: a pool member's budgets must be positive. Zero is not a smaller
    /// setting, it is opting out of the half of the bargain that costs you.
    #[test]
    fn an_app_in_the_pool_carries_a_share_of_it() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for app in ["follower/src/main/kotlin", "plugin/src/main/kotlin"] {
            let mut src = String::new();
            collect_kotlin(&root.join(app), &mut src);
            if !src.contains("SwarmNative.swarmJoin(") {
                continue;
            }
            // The last argument of each call, with comments and whitespace
            // stripped — the budget is written on its own line behind a
            // comment in both apps.
            for call in ["swarmTick", "keysCarryAll"] {
                let Some(at) = src.find(&format!("SwarmNative.{call}(")) else { continue };
                let tail = &src[at..];
                let Some(close) = tail.find(')') else { continue };
                let args: String = tail[..close]
                    .lines()
                    .map(|l| l.split("//").next().unwrap_or("").trim())
                    .collect::<Vec<_>>()
                    .join(" ");
                let last = args.rsplit(',').next().unwrap_or("").trim().to_string();
                assert_ne!(
                    last, "0",
                    "{app} calls {call} with an adoption budget of 0.\n\
                     It joins the pool, announces what it holds, and carries nothing for \n\
                     anybody. A pool whose followers all do this has one holder per subject \n\
                     — the subject — which is the availability this design exists to remove. \n\
                     See D28, and the app's own name."
                );
            }
        }
    }

    /// Every `.kt` under a directory, concatenated, minus the declarations.
    fn collect_kotlin(dir: &std::path::Path, out: &mut String) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.filter_map(|e| e.ok()) {
            let path = e.path();
            if path.is_dir() {
                collect_kotlin(&path, out);
            } else if path.extension().is_some_and(|x| x == "kt") {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    for line in text.lines() {
                        if !line.trim().starts_with("external fun ") {
                            out.push_str(line);
                            out.push('\n');
                        }
                    }
                }
            }
        }
    }

    /// BOTH APPS MUST STILL ASK ANDROID FOR MULTICAST, AND STILL TAKE THE LOCK.
    ///
    /// **THE ONE DEFECT NO RUNTIME TEST IN THIS REPO CAN CATCH.** p2panda spawns
    /// `MdnsDiscovery` in `Active` mode, but on Android the wifi chip does not
    /// deliver multicast to userspace unless an app holds a `MulticastLock`,
    /// and taking one needs `CHANGE_WIFI_MULTICAST_STATE`. p2panda cannot do it
    /// — it is an Android API, so only the app can. A laptop has no such
    /// switch, so every test here passes with mDNS stone deaf.
    ///
    /// It cost a full overnight outage on 2026-09-14: two phones on one wifi,
    /// the subject moved, and neither could find the other. `:5353` with
    /// `Recv-Q 0` is the signature.
    ///
    /// So this reads the source. Crude, and the alternative is finding out on a
    /// phone again — a permission silently dropped in a manifest merge looks
    /// exactly like the network being quiet.
    #[test]
    fn both_apps_still_ask_for_multicast_and_take_the_lock() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for manifest in ["follower/src/main/AndroidManifest.xml", "plugin/src/main/AndroidManifest.xml"] {
            let text = std::fs::read_to_string(root.join(manifest))
                .unwrap_or_else(|e| panic!("{manifest}: {e}"));
            assert!(
                text.contains("android.permission.CHANGE_WIFI_MULTICAST_STATE"),
                "{manifest} no longer asks for CHANGE_WIFI_MULTICAST_STATE — mDNS goes deaf and \
                 nothing on a desktop will tell you"
            );
        }

        // Declaring it does nothing on its own; something has to acquire.
        let shared = root.join("plugin/src/main/kotlin/nz/diaswarm/jni/SwarmNative.kt");
        let text = std::fs::read_to_string(&shared).expect("SwarmNative.kt");
        assert!(
            text.contains("createMulticastLock"),
            "nothing acquires a MulticastLock any more — the permission alone changes nothing"
        );

        // And both entry points have to call it, or one app is deaf.
        for caller in [
            "follower/src/main/kotlin/nz/diaswarm/follower/Endpoint.kt",
            "plugin/src/main/kotlin/app/aaps/plugins/sync/swarm/SwarmPlugin.kt",
        ] {
            let text = std::fs::read_to_string(root.join(caller))
                .unwrap_or_else(|e| panic!("{caller}: {e}"));
            assert!(
                text.contains("Multicast.hold"),
                "{caller} no longer takes the multicast lock before starting its endpoint"
            );
        }
    }

    /// WITHDRAWING HAS TO REACH THE VAULT THE DATA IS IN.
    ///
    /// **A READER IS TWO MEMBERS.** One in the core vault, wrapped per segment,
    /// and one in the keys group named by a derived tag. Granting always did
    /// both — a scan that did only one was fixed the day it was noticed.
    /// Revoking did only the core one, and nothing had kept the tag: it came
    /// back from `keysGrant`, went into a log line, and was dropped. So after
    /// the cutover, withdrawing would have removed somebody from the vault that
    /// no longer holds the data and left them reading the one that does.
    ///
    /// What makes the fix small is D13. The tag is derived from the pair and
    /// the purpose, so the way out can recompute exactly what the way in
    /// created, and the only thing worth storing is the identity it derives
    /// from — one input, two derivations, no second book to fall out of step.
    #[test]
    fn a_withdrawal_names_the_same_member_the_grant_created() {
        use diaswarm_keys::{Vault, encode_identity, decode_identity, KeysIdentity};

        let (mut subject, _store, _rt, _key) = vault("withdraw");
        let rng = diaswarm_keys::Rng::default();

        // A reader as the app sees one: an identity out of an invite.
        let (_reader_mgr, reader_bundle) = Vault::key_bundle(&rng).unwrap();
        let reader_signer = p2panda_core::SigningKey::generate();
        let identity = encode_identity(&KeysIdentity {
            signer: reader_signer.verifying_key(),
            bundle: reader_bundle,
        })
        .unwrap();
        let bundle = decode_identity(&identity).unwrap().bundle;

        let (_welcome, granted) = subject.grant(bundle.clone(), "follow").expect("grant");

        // The tag is never stored. Recomputing it from the identity alone —
        // which is all a withdrawal has — must name the member that was added.
        let named = subject.tag_of(bundle.clone(), "follow").expect("derive");
        assert_eq!(named, granted, "a withdrawal would have removed the wrong member");

        subject.revoke(named).expect("revoke");

        // And they are out: a handover cannot put them back, and the derivation
        // still agrees so a second withdrawal is not an error either.
        assert!(
            matches!(
                subject.grant_unattended(bundle.clone(), "follow"),
                Err(diaswarm_keys::Error::Revoked(_))
            ),
            "a revoked reader was let back in by a handover"
        );
        assert_eq!(subject.tag_of(bundle, "follow").unwrap(), granted);
    }

    /// A CANCELLED TEMPORARY TARGET IS A RECORD, NOT AN ABSENCE.
    ///
    /// AAPS calls a temporary target off by writing another one with zero
    /// duration. So "newest wins" is not just the right rule for picking
    /// between two live targets — it is the ONLY rule that sees a cancellation
    /// at all. A reader that took the newest target with a non-zero duration
    /// would happily show an exercise target somebody switched off an hour ago.
    #[test]
    fn cancelling_a_temporary_target_is_the_newest_record_not_a_missing_one() {
        use super::newest_of_kind;
        use diaswarm_core::Record;

        let tt = |t: i64, dur: f64| {
            Record::new(t, "target")
                .set("lo", Some(80.0.into()))
                .set("hi", Some(140.0.into()))
                .set("dur", Some(dur.into()))
                .set("why", Some("ACTIVITY".into()))
                .to_canonical_json()
        };
        // Set at 1000 for 30 minutes, cancelled at 2000. Out of order on
        // purpose: epochs are concatenated, so arrival order proves nothing.
        let body = [tt(2_000, 0.0), tt(1_000, 1_800_000.0)].join("\n");
        let got = newest_of_kind(&body, "target").expect("no target found");
        assert!(got.contains("\"dur\":0"), "picked the set, not the cancel: {got}");

        // And the profile is a different statement read by the same helper —
        // one body, two kinds, neither answering for the other.
        let mixed = [
            tt(1_000, 1_800_000.0),
            Record::new(5_000, "profile").set("basal", Some(serde_json::json!([]))).to_canonical_json(),
        ]
        .join("\n");
        assert!(newest_of_kind(&mixed, "target").unwrap().contains("ACTIVITY"));
        assert!(newest_of_kind(&mixed, "profile").unwrap().contains("basal"));
        assert_eq!(newest_of_kind(&mixed, "cgm"), None);
    }

    /// THE ROW SHAPE IS THE CONTRACT, AND BOTH VAULTS READ THE SAME ONE.
    ///
    /// `keysTreatments` and `netTreatments` build their rows from this one
    /// function precisely so a subject moving between vaults cannot change
    /// what their chart draws. This pins the shape so the two cannot be
    /// separated by an edit to one of them — which is the only way they could
    /// drift now.
    ///
    /// `abs` is the field worth the test. It is not a value, it decides what
    /// `rate` *means*: 150 is either 150% of basal or 150 U/h, and a chart
    /// given the number without the flag has no way to tell.
    #[test]
    fn a_treatment_row_says_the_same_thing_whichever_vault_read_it() {
        use super::treatment_line;
        use diaswarm_core::Record;

        let row = |r: Record| treatment_line(&r).expect("a known kind produced no row");

        assert_eq!(
            row(Record::new(1_000, "bolus").set("u", Some(2.5.into())).set("type", Some("SMB".into()))),
            "bolus\t1000\t2.5\t0\tSMB"
        );
        assert_eq!(
            row(Record::new(2_000, "carb").set("g", Some(30.0.into()))),
            "carb\t2000\t30\t0\t"
        );
        assert_eq!(
            row(Record::new(3_000, "tbr").set("rate", Some(150.0.into())).set("dur", Some(30.0.into()))),
            "tbr\t3000\t150\t30\t",
            "a percentage TBR must not be labelled absolute"
        );
        assert_eq!(
            row(Record::new(3_000, "tbr")
                .set("rate", Some(1.5.into()))
                .set("dur", Some(30.0.into()))
                .set("abs", Some(true.into()))),
            "tbr\t3000\t1.5\t30\tabs",
            "an absolute TBR that loses its flag reads as a percentage"
        );
        assert_eq!(
            row(Record::new(4_000, "extbolus").set("u", Some(1.0.into())).set("dur", Some(60.0.into()))),
            "extbolus\t4000\t1\t60\t"
        );

        // Everything else belongs to the glucose reader or to nobody, and must
        // not be smuggled onto the treatment chart as a malformed row.
        assert!(treatment_line(&Record::new(5_000, "cgm").set("mgdl", Some(100.0.into()))).is_none());
        assert!(treatment_line(&Record::new(5_000, "loop").set("iob", Some(1.0.into()))).is_none());
    }

    /// THE PROFILE IN FORCE IS THE LATEST ONE, NOT THE LAST ONE READ.
    ///
    /// Segments are opened epoch by epoch and concatenated, so the order rows
    /// arrive in is the order their days were opened — which is not the order
    /// they were written, and a reader taking the last line would pick a
    /// profile by which day happened to be read last.
    ///
    /// The stake is not cosmetic: the basal blocks in this record are what a
    /// percentage temp basal is a percentage *of*.
    #[test]
    fn the_newest_profile_wins_whatever_order_the_epochs_arrived_in() {
        use super::newest_profile;
        use diaswarm_core::Record;

        let profile = |t: i64, rate: f64| {
            Record::new(t, "profile")
                .set("basal", Some(serde_json::json!([{ "duration": 86_400_000, "amount": rate }])))
                .to_canonical_json()
        };
        let body = [
            profile(3_000, 0.9),
            Record::new(9_000, "cgm").set("mgdl", Some(100.0.into())).to_canonical_json(),
            profile(5_000, 0.45),
            profile(1_000, 0.3),
        ]
        .join("\n");

        let got = newest_profile(&body).expect("no profile found");
        assert!(got.contains("0.45"), "picked the wrong profile: {got}");
        assert_eq!(newest_profile(""), None);
        assert_eq!(
            newest_profile(&Record::new(1, "cgm").set("mgdl", Some(90.0.into())).to_canonical_json()),
            None,
            "a body with no profile in it must answer None, not a fabricated one"
        );
    }
}
