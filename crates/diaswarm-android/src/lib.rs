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
) -> jlong {
    epoch_of(t)
}

/// The stream header (spec §5.2), as a canonical JSON line.
#[no_mangle]
pub extern "system" fn Java_app_aaps_plugins_sync_swarm_SwarmNative_header<'a>(
    env: JNIEnv<'a>,
    _class: JClass<'a>,
) -> JString<'a> {
    to_jstring(env, header().to_canonical_json())
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

fn to_jstring(env: JNIEnv<'_>, s: String) -> JString<'_> {
    env.new_string(s).unwrap_or_else(|_| unsafe { JString::from_raw(std::ptr::null_mut()) })
}
