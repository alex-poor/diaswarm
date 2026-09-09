//! The JNI surface the AAPS plugin calls.
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

/// Which version of `spec/records.md` this build implements.
///
/// The plugin should refuse to publish if this disagrees with what it expects:
/// a silently mismatched native library is how a stream ends up conforming to a
/// spec nobody thinks it conforms to.
#[no_mangle]
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_specVersion(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    diaswarm_core::SPEC_VERSION as jint
}

/// Which epoch a timestamp falls in — the unit of key custody, a UTC day.
#[no_mangle]
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_epochOf(
    _env: JNIEnv,
    _class: JClass,
    t: jlong,
    offset_ms: jlong,
) -> jlong {
    epoch_of(t, offset_ms)
}

/// The stream header (spec §5.2), as a canonical JSON line.
#[no_mangle]
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_header<'a>(
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
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_canonicalLine<'a>(
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
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_emitterNew(
    _env: JNIEnv,
    _class: JClass,
) -> jlong {
    Box::into_raw(Box::new(Emitted::new())) as jlong
}

#[no_mangle]
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_emitterFree(
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
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_emitterAccept<'a>(
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
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_emitterAmendments(
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
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_vaultSeal<'a>(
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
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_vaultSubject<'a>(
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
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_vaultStatus<'a>(
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
}

/// Join the pool. Returns a handle, or 0.
///
/// Uses the SAME node key file as before, so the phone keeps the endpoint id it
/// already had. Everything that ranks peers ranks them by that id, and a new
/// key would read as one peer leaving and another arriving — every bucket it
/// held would reshuffle for nothing.
#[no_mangle]
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_swarmJoin<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    store_path: JString<'a>,
    node_key_path: JString<'a>,
) -> jlong {
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

    Box::into_raw(Box::new(Pooled { runtime, swarm, node_id })) as jlong
}

/// This phone's id in the pool. Empty on a bad handle.
#[no_mangle]
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_swarmNodeId<'a>(
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

/// One pass: say what we hold, hear what we should, take on a few of them.
///
/// Returns `pool<TAB>buckets<TAB>held<TAB>wanted<TAB>adopted`, or empty.
#[no_mangle]
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_swarmTick<'a>(
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
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_swarmLeave<'a>(
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
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_netFollow<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    store_path: JString<'a>,
    invite_text: JString<'a>,
) -> jlong {
    let (Ok(store), Ok(text)) = (env.get_string(&store_path), env.get_string(&invite_text)) else {
        return -1;
    };
    let Ok(inv) = diaswarm_core::invite::Invite::parse(&String::from(text)) else { return -2 };
    match diaswarm_net::peer::add_follow(
        Path::new(&String::from(store)),
        &inv.subject,
        &inv.endpoint,
        Some(&inv.purpose),
    ) {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(_) => -3,
    }
}

/// Bring every followed subject up to date. Returns how many were reached, or
/// a negative code. Blocking: the caller is already a worker thread.
#[no_mangle]
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_netRefresh<'a>(
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
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_netFollowing<'a>(
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
                "{}\t{}\t{}",
                f.subject,
                f.purpose.clone().unwrap_or_else(|| "relay".into()),
                if held { "1" } else { "0" }
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
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_netLatest<'a>(
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

/// Who this subject has granted, one per line: `reader<TAB>purpose`.
///
/// Read from the vault's private book, which is the only thing that can put a
/// name to a tag — the grant log deliberately cannot (D13). Empty when nobody
/// has been granted, which is indistinguishable here from a vault that does
/// not exist; both mean "nothing to show", and the caller knows which.
#[no_mangle]
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_vaultReaders<'a>(
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
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_inviteFor<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    subject: JString<'a>,
    endpoint: JString<'a>,
    purpose: JString<'a>,
) -> JString<'a> {
    let (Ok(s), Ok(e), Ok(p)) =
        (env.get_string(&subject), env.get_string(&endpoint), env.get_string(&purpose))
    else {
        return to_jstring(env, String::new());
    };
    match diaswarm_core::invite::Invite::new(&String::from(s), &String::from(e), &String::from(p)) {
        Ok(inv) => to_jstring(env, inv.encode()),
        Err(_) => to_jstring(env, String::new()),
    }
}

/// Read an invite, returning `subject\tendpoint\tpurpose`, or empty if it is
/// not one. Lets the phone accept an invite from another subject later.
#[no_mangle]
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_inviteParse<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    text: JString<'a>,
) -> JString<'a> {
    let Ok(t) = env.get_string(&text) else { return to_jstring(env, String::new()) };
    match diaswarm_core::invite::Invite::parse(&String::from(t)) {
        Ok(i) => to_jstring(env, format!("{}\t{}\t{}", i.subject, i.endpoint, i.purpose)),
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
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_vaultRewrap<'a>(
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
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_vaultGrant<'a>(
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
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_vaultRevoke<'a>(
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
