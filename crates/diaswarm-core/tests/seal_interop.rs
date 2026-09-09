//! Does the Rust sealing agree with `tools/seal.py`?
//!
//! Not "does it look the same" — these seal in one language and open in the
//! other, in both directions, on the same wire bytes. A construction that two
//! implementations describe identically and encode differently is exactly the
//! failure this project has already hit twice, and it is invisible until two
//! peers cannot read each other.

use std::process::Command;

use diaswarm_core::seal::{epoch_key, open_epoch, seal_epoch, unwrap, wrap, SealError};
use x25519_dalek::{PublicKey, StaticSecret};

const EPOCH: i64 = 20_663;

fn python(args: &[&str]) -> Option<serde_json::Value> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)?
        .to_path_buf();
    let out = Command::new("python3")
        .arg(root.join("crates/diaswarm-core/tests/interop.py"))
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        eprintln!("python: {}", String::from_utf8_lossy(&out.stderr));
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

fn hex_to_32(s: &str) -> [u8; 32] {
    let bytes: Vec<u8> = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect();
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    out
}

fn hex_to_vec(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn rust_opens_what_python_sealed() {
    let secret = StaticSecret::random_from_rng(rand_core::OsRng);
    let public = PublicKey::from(&secret);

    let Some(v) = python(&["seal", &to_hex(public.as_bytes()), &EPOCH.to_string()]) else {
        eprintln!("skipping: python3 or the repo tools are unavailable");
        return;
    };

    let subject = hex_to_32(v["subject_pub"].as_str().unwrap());
    let sealed = hex_to_vec(v["sealed"].as_str().unwrap());
    let wrapped = hex_to_vec(v["wrap"].as_str().unwrap());

    let key = unwrap(&wrapped, EPOCH, &secret).expect("the wrap is addressed to us");
    let plaintext = open_epoch(&sealed, &key, EPOCH, &subject).expect("the key opens the epoch");

    let text = String::from_utf8(plaintext).unwrap();
    assert_eq!(
        text.trim(),
        r#"{"k":"cgm","mgdl":163.0,"src":"Dexcom G6","t":1782938503230,"trend":"FLAT"}"#,
        "Rust opened Python's epoch but got different bytes out"
    );
}

#[test]
fn python_opens_what_rust_sealed() {
    let Some(reader) = python(&["reader"]) else { return };
    let reader_pub = hex_to_32(reader["public"].as_str().unwrap());
    let reader_secret = reader["secret"].as_str().unwrap().to_string();

    let subject = [7u8; 32];
    let key = epoch_key();
    let record = r#"{"k":"cgm","mgdl":163.0,"src":"Dexcom G6","t":1782938503230,"trend":"FLAT"}"#;
    let plaintext = format!("{record}\n");

    let sealed = seal_epoch(plaintext.as_bytes(), &key, EPOCH, &subject);
    let wrapped = wrap(&key, EPOCH, &reader_pub);

    let Some(v) = python(&[
        "open",
        &reader_secret,
        &EPOCH.to_string(),
        &to_hex(&subject),
        &to_hex(&sealed),
        &to_hex(&wrapped),
    ]) else {
        panic!("python could not open what Rust sealed");
    };

    assert_eq!(v[0]["mgdl"].as_f64(), Some(163.0));
    assert_eq!(v[0]["k"].as_str(), Some("cgm"));
}

#[test]
fn a_wrap_for_one_reader_does_not_open_for_another() {
    let mine = StaticSecret::random_from_rng(rand_core::OsRng);
    let theirs = StaticSecret::random_from_rng(rand_core::OsRng);
    let key = epoch_key();
    let wrapped = wrap(&key, EPOCH, PublicKey::from(&mine).as_bytes());

    assert!(matches!(unwrap(&wrapped, EPOCH, &mine), Ok(_)));
    assert!(
        matches!(unwrap(&wrapped, EPOCH, &theirs), Err(SealError::Undecryptable)),
        "a wrap addressed to one reader opened for another"
    );
}

#[test]
fn a_sealed_epoch_cannot_be_passed_off_as_another() {
    let subject = [7u8; 32];
    let key = epoch_key();
    let sealed = seal_epoch(b"records\n", &key, EPOCH, &subject);

    assert!(open_epoch(&sealed, &key, EPOCH, &subject).is_ok());
    assert!(
        open_epoch(&sealed, &key, EPOCH + 1, &subject).is_err(),
        "epoch is authenticated, so moving it must fail"
    );
    assert!(
        open_epoch(&sealed, &key, EPOCH, &[9u8; 32]).is_err(),
        "subject is authenticated, so re-attributing it must fail"
    );
}

#[test]
fn a_wrap_is_the_size_the_design_was_costed_on() {
    let key = epoch_key();
    let secret = StaticSecret::random_from_rng(rand_core::OsRng);
    let w = wrap(&key, EPOCH, PublicKey::from(&secret).as_bytes());
    // §7.2 assumed ~100 bytes a wrap when it priced key records at 180 KB/year.
    assert_eq!(w.len(), 92, "wrap size moved; §7.2's costing assumed 92");
}
