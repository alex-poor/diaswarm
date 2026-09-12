//! Does the SHIPPING vault get dearer as history grows?
//!
//! **THE CONTROL FOR `diaswarm-spaces/src/bin/opcost.rs`.** That measured the
//! spaces vault and found the marginal cost of one operation rising with
//! everything already processed — about 21 µs per operation of history — because
//! application messages chain to their space's previous tips and the decryption
//! state ratchets forward. A reader who only ever displays a day still has to
//! walk the whole chain to reach it.
//!
//! The claim that followed was that `diaswarm-core` does not behave this way:
//! segments are sealed independently, per epoch, and a reader opens the ones it
//! holds a wrap for. If that is right, reading the recent end should cost the
//! same whether the vault holds a week or a year, and the migration has a
//! problem the shipped code does not.
//!
//! That claim is worth a measurement rather than an assertion, which is what
//! this is.
//!
//! ```sh
//! cargo run --release --bin readcost
//! ```

use std::path::PathBuf;
use std::time::Instant;

use diaswarm_core::vault::{Identity, Vault};
use diaswarm_core::{EPOCH_MS, Record};

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-readcost-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// A day of a five-minute sensor, which is what the storage figures assume.
fn day(epoch: i64) -> Vec<Record> {
    (0..288i64)
        .map(|i| {
            Record::new(epoch * EPOCH_MS + i * 300_000, "cgm")
                .set("mgdl", Some((100.0 + (i % 80) as f64).into()))
        })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("  {:>8}  {:>12}  {:>14}  {:>10}", "days held", "whole vault", "recent day only", "records");

    for days in [7i64, 30, 90, 180] {
        let root = tmp("vault");
        let subject = Identity::generate();
        let reader = Identity::generate();
        let vault = Vault::create(&root, &subject, 12 * 3_600_000).expect("create");

        for e in 0..days {
            vault.seal(20_000 + e, &day(20_000 + e)).expect("seal");
        }
        // The same two calls the JNI's vaultGrant makes.
        let reader_pub: [u8; 32] = x25519_dalek::PublicKey::from(&reader.encryption).to_bytes();
        vault.record_grant(&subject, &reader_pub, "follow", "grant", 0).expect("record");
        vault.publish_wraps(&subject, &reader_pub, "follow").expect("wraps");

        // Everything the reader can open — what a catch-up costs.
        let t0 = Instant::now();
        let all = vault.read_as(&reader, "follow").expect("read all");
        let whole = t0.elapsed();

        // Only the newest epoch — what a parent watching 24 hours costs, and
        // the case the spaces vault cannot express.
        let newest = 20_000 + days - 1;
        let t1 = Instant::now();
        let recent = vault.read_as_from(&reader, "follow", newest).expect("read recent");
        let last = t1.elapsed();

        println!(
            "  {:>8}  {:>11.3}s  {:>13.3}s  {:>10}",
            days,
            whole.as_secs_f64(),
            last.as_secs_f64(),
            all.values().map(Vec::len).sum::<usize>()
        );
        let _ = recent;
    }

    println!();
    println!("  If the last column is flat, a reader of the recent end pays the same");
    println!("  whatever the subject holds, and the shipped vault does not have the");
    println!("  problem the spaces one does.");
    Ok(())
}
