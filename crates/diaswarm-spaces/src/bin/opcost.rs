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

use std::borrow::Borrow;
use std::path::PathBuf;
use std::time::Instant;

use diaswarm_core::{EPOCH_MS, Record};
use diaswarm_spaces::{Conditions, Reach, SpacesArgs, Vault};

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
    // DOES A FRESH READER ACTUALLY START CHEAP? The recommendation that falls
    // out of the curve above is "forgetting, not pruning": a parent needs no
    // history, so a reader periodically re-granted into a new window should
    // start clean. That is a hypothesis about where the state lives, and it is
    // worth one measurement before anyone designs around it.
    println!();
    println!("  a NEW reader joining a subject that already has history:");
    {
        let mut subject = Vault::open(tmp("aged-subject"), OFFSET).await?;
        let early_dir = tmp("aged-early");
        let early = Vault::open(early_dir.clone(), OFFSET).await?;
        subject.register(&early).await?;
        early.register(&subject).await?;

        let mut ops = Vec::new();
        ops.extend(subject.seal(&[]).await?);
        ops.extend(subject.grant(early.subject(), Reach::Everything).await?);
        let mut n = 0i64;
        while ops.len() < 1500 {
            let t = 24_000 * EPOCH_MS + n * 60_000;
            ops.extend(subject.seal(&[reading(t, 100.0 + (n % 80) as f64)]).await?);
            n += 1;
        }
        println!("    subject now has {} operations of history", ops.len());

        // A second reader, granted FROM NOW: a window that cannot contain any
        // of the above.
        let late = Vault::open(tmp("aged-late"), OFFSET).await?;
        subject.register(&late).await?;
        late.register(&subject).await?;

        // THE AUTH HISTORY IS NOT OPTIONAL, AND IT IS NOT THE BULK. All of a
        // subject's spaces share one global auth state (D20), so a reader
        // granted into a brand-new window still needs every auth-carrying
        // operation — the window creations and the grants. What it does NOT
        // need is the application messages, which are the thousands.
        //
        // A first attempt handed the new reader only its own window and got
        // "held 584": every operation waiting on an auth dependency it had
        // never been given. That is the orderer reporting a test bug correctly.
        let auth: Vec<_> = ops
            .iter()
            .filter(|o| {
                !matches!(
                    Borrow::<SpacesArgs<Conditions>>::borrow(*o),
                    SpacesArgs::Application { .. }
                )
            })
            .cloned()
            .collect();
        println!(
            "    of which auth-carrying: {} ({} are application messages)",
            auth.len(),
            ops.len() - auth.len()
        );
        let auth_only = auth.clone();
        let mut fresh = auth;
        fresh.extend(subject.grant(late.subject(), Reach::FromNow).await?);

        // One day's worth for each of them, and time both.
        let mut day = Vec::new();
        for i in 0..288i64 {
            let t = 24_100 * EPOCH_MS + i * 300_000;
            day.extend(subject.seal(&[reading(t, 110.0 + (i % 60) as f64)]).await?);
        }
        fresh.extend(day.clone());
        ops.extend(day.clone());

        let t0 = Instant::now();
        let a = early.ingest(&ops).await?;
        let aged = t0.elapsed();

        let t1 = Instant::now();
        let b = late.ingest(&fresh).await?;
        let clean = t1.elapsed();

        println!(
            "    reader with all the history : {:>7.2}s for {} records",
            aged.as_secs_f64(),
            a.records.len()
        );
        println!(
            "    reader granted from now     : {:>7.2}s for {} records",
            clean.as_secs_f64(),
            b.records.len()
        );
        println!(
            "      aged  refused {} held {} panicked {}   |   fresh ops {} refused {} held {} panicked {}",
            a.refused, a.held, a.panicked,
            fresh.len(), b.refused, b.held, b.panicked
        );
        println!();

        // THE QUESTION THE ABOVE DOES NOT ANSWER, AND THE ONE THAT MATTERS.
        // `late` was a brand-new vault. A parent who has followed for a year is
        // not: their accumulated state is in their own spaces.sqlite, and a new
        // grant does not touch it. So can an EXISTING reader shed that state and
        // keep its grant?
        //
        // The identity is `credentials.json`; the state is `spaces.sqlite`.
        // Dropping the second and keeping the first leaves the same member with
        // no memory — which only works if everything it needs can be handed to
        // it again, and in a swarm it can, because the operations are in the
        // pool.
        println!("  the SAME aged reader, after discarding its state:");
        let aged_dir = early_dir.clone();
        drop(early);
        for f in ["spaces.sqlite", "spaces.sqlite-shm", "spaces.sqlite-wal"] {
            let _ = std::fs::remove_file(aged_dir.join(f));
        }
        let reborn = Vault::open(aged_dir, OFFSET).await?;
        reborn.register(&subject).await?;

        // Auth chain plus one day — what a pool would hand back.
        let mut catchup = auth_only.clone();
        catchup.extend(day.clone());
        let t2 = Instant::now();
        let c = reborn.ingest(&catchup).await?;
        println!(
            "    kept identity, dropped state: {:>7.2}s for {} records (held {})",
            t2.elapsed().as_secs_f64(),
            c.records.len(),
            c.held
        );
        println!();

        // THE ONE THAT ACTUALLY MATCHES THE USE CASE. A parent follows their
        // child permanently and reads a day. Nothing above tests the obvious
        // move: the SUBJECT opens a new window and adds the same parent to it.
        // No scan, no new key, no re-pair — the subject already holds their
        // key. If spaces state is per-space, the parent processes the new
        // window from its own beginning and the old one stops mattering.
        println!("  the SAME long-standing reader, moved to a new window by the subject:");
        let rotated = Vault::open(tmp("rotated-reader"), OFFSET).await?;
        subject.register(&rotated).await?;
        rotated.register(&subject).await?;

        // Give them the aged experience first: a full history, processed.
        let mut theirs = auth_only.clone();
        theirs.extend(subject.grant(rotated.subject(), Reach::Everything).await?);
        theirs.extend(day.clone());
        let warm = rotated.ingest(&theirs).await?;
        println!("    after a long stretch of following: {} records held", warm.records.len());

        // Now the subject rotates: a new window, same reader added to it.
        let rotate = subject.grant(rotated.subject(), Reach::FromNow).await?;
        let mut after = rotate;
        for i in 0..288i64 {
            let t = 24_200 * EPOCH_MS + i * 300_000;
            after.extend(subject.seal(&[reading(t, 120.0 + (i % 40) as f64)]).await?);
        }

        let t3 = Instant::now();
        let d = rotated.ingest(&after).await?;
        println!(
            "    a day in the NEW window     : {:>7.2}s for {} records (held {})",
            t3.elapsed().as_secs_f64(),
            d.records.len(),
            d.held
        );
        println!();
        println!("  Cheap and reading means the subject can rotate windows on a schedule,");
        println!("  the parent never re-pairs, and the cost resets. Expensive means state");
        println!("  is shared across a subject's spaces and rotation buys nothing.");
    }
    Ok(())
}
