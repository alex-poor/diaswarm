//! Does the spaces vault actually work on a phone?
//!
//! WHY A BINARY AND NOT A JNI CALL. Everything measured so far ran on a laptop.
//! The stack underneath `diaswarm-spaces` is new to this project on Android —
//! a bundled SQLite writing to app storage, a tokio runtime, and p2panda's
//! state machinery — and "it cross-compiles" is not "it runs". The cheapest
//! honest test is a binary pushed to `/data/local/tmp` and run over adb, which
//! touches AAPS not at all: no install, no plugin, nothing near a pump.
//!
//! ```sh
//! cargo ndk -t arm64-v8a build --release --bin selftest
//! adb push target/aarch64-linux-android/release/selftest /data/local/tmp/
//! adb shell /data/local/tmp/selftest /data/local/tmp/diaswarm-selftest
//! ```
//!
//! It exercises the whole lifecycle a phone would: create a vault, seal days,
//! grant a reader, have that reader open what it was given, revoke, and check
//! the revocation held. Then it reopens the vault from disk, because a phone
//! restarts and an identity that does not survive that invalidates every grant
//! ever made to it.

use std::path::PathBuf;
use std::time::Instant;

use diaswarm_core::{EPOCH_MS, Record, kind};
use diaswarm_spaces::{Reach, Vault};

const OFFSET: i64 = 12 * 3_600_000;

/// A day of five-minute readings, which is the shape a real day has.
fn day(epoch: i64) -> Vec<Record> {
    (0..288i64)
        .map(|i| {
            Record::new(epoch * EPOCH_MS + i * 300_000, "cgm")
                .set("mgdl", Some((90.0 + (i as f64 * 1.7) % 130.0).into()))
        })
        .collect()
}

fn ok(label: &str, pass: bool, detail: &str) -> bool {
    println!("  {:<42} {}  {}", label, if pass { "ok  " } else { "FAIL" }, detail);
    pass
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root: PathBuf = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/data/local/tmp/diaswarm-selftest".to_string())
        .into();
    // A clean run every time: a half-written vault from a previous attempt
    // would otherwise look like a defect in this one.
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root)?;

    println!("\n  diaswarm-spaces self-test");
    println!("  {}\n", std::env::consts::ARCH);

    let mut all = true;
    let t = Instant::now();
    let mut subject = Vault::open(root.join("subject"), OFFSET).await?;
    let reader = Vault::open(root.join("reader"), OFFSET).await?;
    all &= ok("opens two vaults", true, &format!("{:?}", t.elapsed()));

    subject.register(&reader).await?;
    reader.register(&subject).await?;

    // --- seal a week, granting before the last two days ------------------
    let t = Instant::now();
    let mut ops = subject.seal(&[]).await?;
    let mut sealed = 0usize;
    for e in 0..5i64 {
        let d = day(22_000 + e);
        sealed += d.len();
        ops.extend(subject.seal(&d).await?);
    }
    let sealing = t.elapsed();
    all &= ok(
        "seals five days",
        true,
        &format!("{sealed} records, {} operations, {sealing:?}", ops.len()),
    );

    let t = Instant::now();
    ops.extend(subject.grant(reader.subject(), Reach::Everything).await?);
    for e in 5..7i64 {
        let d = day(22_000 + e);
        sealed += d.len();
        ops.extend(subject.seal(&d).await?);
    }
    all &= ok("grants a reader", true, &format!("{:?}", t.elapsed()));

    // --- the reader opens the lot ----------------------------------------
    let t = Instant::now();
    let got = reader.ingest(&ops).await?;
    let reading = t.elapsed();
    // Without the stream header, which is emitted per window rather than
    // sealed by the caller and would otherwise make this off by one.
    let opened = got.records.iter().filter(|r| r.kind() != kind::META).count();
    all &= ok(
        "reader opens every record",
        opened == sealed && got.panicked == 0,
        &format!("{opened} of {sealed}, {} panicked, {reading:?}", got.panicked),
    );

    // --- revocation is prospective ---------------------------------------
    let before = opened;
    let mut after_ops = subject.revoke(reader.subject()).await?;
    let last = day(22_100);
    after_ops.extend(subject.seal(&last).await?);
    let after = reader.ingest(&after_ops).await?;
    let leaked = after.records.iter().filter(|r| r.kind() != kind::META).count();
    all &= ok(
        "revocation stops the next day",
        leaked == 0,
        &format!("{leaked} records after revoking (want 0), kept {before}"),
    );

    // --- and the identity survives a restart ------------------------------
    let windows = subject.windows();
    let subject_id = subject.subject();
    drop(subject);
    let reopened = Vault::open(root.join("subject"), OFFSET).await?;
    all &= ok(
        "identity survives a restart",
        reopened.subject() == subject_id && reopened.windows() == windows,
        "same key, same windows",
    );

    let bytes: u64 = walk(&root);
    println!("\n  on disk: {:.1} MB\n", bytes as f64 / 1e6);
    println!("  {}\n", if all { "ALL OK" } else { "SOMETHING FAILED" });
    if !all {
        std::process::exit(1);
    }
    Ok(())
}

fn walk(dir: &PathBuf) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|e| {
            let p = e.path();
            if p.is_dir() { walk(&p) } else { p.metadata().map(|m| m.len()).unwrap_or(0) }
        })
        .sum()
}
