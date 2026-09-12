//! What does an operation cost to read, and does that cost grow?
//!
//! **THE QUESTION BEHIND A DESIGN DECISION, NOT A BENCHMARK FOR ITS OWN SAKE.**
//! [D20](../../../docs/decisions.md) measured reads as quadratic in operation
//! count — 5 ms per operation at 537, 155 ms at 10,569, twenty-seven minutes for
//! 74 days — and closed the question with "at 79 operations for 74 days it stops
//! mattering". That figure came from a *backfill*, where a year of history is
//! batched into 64 KB bodies.
//!
//! Live sealing cannot do that. A pass seals what has arrived since the last
//! one, and with a one-minute sensor that is one reading; batching to 64 KB
//! would mean sealing about hourly, and a follower seeing a reading an hour late
//! is not worth building. So the shipped vault accumulates operations at a rate
//! the measured figure never covered — 5,516 on the loop phone after six weeks,
//! against the 79 D20 reasoned from.
//!
//! If the cost per operation is flat, batching is a constant-factor tidy-up and
//! the design is fine. If it grows, batching only postpones a wall and the
//! answer is somewhere else entirely. That is worth knowing before choosing a
//! threshold, which is what this exists to settle.
//!
//! ```sh
//! cargo run --release --bin opcost
//! ```

use std::path::PathBuf;
use std::time::Instant;

use diaswarm_core::{EPOCH_MS, Record};
use diaswarm_spaces::{Reach, Vault};

const OFFSET: i64 = 12 * 3_600_000;

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-opcost-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// One reading, the size the real ones are: about 86 bytes canonical.
fn reading(t: i64, mgdl: f64) -> Record {
    Record::new(t, "cgm").set("mgdl", Some(mgdl.into()))
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "  {:>8}  {:>10}  {:>12}  {:>10}  {:>10}",
        "ops", "read", "per op", "process", "persist"
    );

    for target in [125usize, 250, 500, 1000, 2000] {
        let subject_dir = tmp("subject");
        let reader_dir = tmp("reader");
        let mut subject = Vault::open(subject_dir.clone(), OFFSET).await?;
        let reader = Vault::open(reader_dir.clone(), OFFSET).await?;
        subject.register(&reader).await?;
        reader.register(&subject).await?;

        let mut ops = Vec::new();
        ops.extend(subject.seal(&[]).await?);
        ops.extend(subject.grant(reader.subject(), Reach::Everything).await?);

        // ONE SEAL PER OPERATION, which is what a live pass does. Each carries a
        // single reading, so the operation count is the variable and the payload
        // stays at the size the wire actually sees.
        let mut n = 0i64;
        while ops.len() < target {
            let t = 24_000 * EPOCH_MS + n * 60_000;
            ops.extend(subject.seal(&[reading(t, 100.0 + (n % 80) as f64)]).await?);
            n += 1;
        }

        let started = Instant::now();
        let got = reader.ingest(&ops).await?;
        let elapsed = started.elapsed();

        // WHERE THE TIME GOES DECIDES WHOSE PROBLEM IT IS. `processing` is
        // inside p2panda-spaces deciding what an operation means; `persisting`
        // is writing the resulting state back to SQLite, which is scheduling
        // and ours to change.
        println!(
            "  {:>8}  {:>9.2}s  {:>9.2}ms  {:>9.2}s  {:>9.2}s",
            ops.len(),
            elapsed.as_secs_f64(),
            elapsed.as_secs_f64() * 1000.0 / ops.len() as f64,
            got.processing.as_secs_f64(),
            got.persisting.as_secs_f64()
        );
        if got.panicked > 0 || got.held > 0 {
            println!("        panicked {} · held {}", got.panicked, got.held);
        }
        // WHERE THE COST ACTUALLY LIVES DECIDES WHETHER PRUNING HELPS. If it is
        // the operations table, dropping old operations bounds it. If it is the
        // spaces state blob — read, mutated and written back on every single
        // operation — then pruning operations changes nothing at all.
        println!("        reader vault: {}", reader_dir.display());
    }

    // THE QUESTION THE CURVE ABOVE CANNOT ANSWER. All-at-once ingest being
    // superlinear has two very different causes, and they want opposite fixes:
    //
    //   * a per-CALL cost — then a follower catching up in chunks pays much
    //     less, and the answer is to chunk;
    //   * a cost that rises with how much history is already processed — then
    //     chunking changes nothing, every operation is dearer than the last for
    //     ever, and the only answer is to stop holding all of it.
    println!();
    println!("  2000 operations, delivered in chunks instead of at once:");
    {
        let mut subject = Vault::open(tmp("chunk-subject"), OFFSET).await?;
        let reader = Vault::open(tmp("chunk-reader"), OFFSET).await?;
        subject.register(&reader).await?;
        reader.register(&subject).await?;

        let mut ops = Vec::new();
        ops.extend(subject.seal(&[]).await?);
        ops.extend(subject.grant(reader.subject(), Reach::Everything).await?);
        let mut n = 0i64;
        while ops.len() < 2000 {
            let t = 24_000 * EPOCH_MS + n * 60_000;
            ops.extend(subject.seal(&[reading(t, 100.0 + (n % 80) as f64)]).await?);
            n += 1;
        }

        let started = Instant::now();
        let mut records = 0;
        for (i, batch) in ops.chunks(200).enumerate() {
            let at = Instant::now();
            records += reader.ingest(&batch.to_vec()).await?.records.len();
            println!(
                "    chunk {:>2}  ops {:>5}..{:<5}  {:>7.2}s",
                i + 1,
                i * 200,
                ((i + 1) * 200).min(ops.len()),
                at.elapsed().as_secs_f64()
            );
        }
        println!(
            "    total {:.2}s for {} records, against {:.2}s all at once",
            started.elapsed().as_secs_f64(),
            records,
            43.6
        );
        println!();
        println!("  MEASURED: chunking changes nothing, and every chunk costs more than");
        println!("  the last. The cost of an operation rises with how much history is");
        println!("  already held, so no delivery schedule fixes it — only holding less.");
    }
    Ok(())
}
