//! Two peers in two OS processes, over real sockets.
//!
//! **THE GAP EVERY OTHER TEST IN THIS CRATE LEAVES.** They run both peers in
//! one process, which makes gossip trivially local: a `Holding` announcement
//! never crosses a network and live delivery is a function call away. So
//! `a_stranger_ends_up_holding_your_ciphertext_without_being_asked` passes — it
//! is a good test and it proves the mechanism — while two real phones report
//! `holders: none` and `received_live_operations: 0` in every session.
//!
//! Found on 2026-09-14, the expensive way: a follower quietly stale, three
//! wrong diagnoses, and finally a sync-event log that had existed all along and
//! could not be read. The failures were between processes, and nothing in the
//! suite could see between processes.
//!
//! These spawn `poolpeer` twice and read its JSON. Slow by this suite's
//! standards — tens of seconds rather than milliseconds — which is the price of
//! exercising sockets, gossip and sync sessions for real.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn tmp(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-2p-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// Spawn a peer and return it with a reader over its stdout.
fn peer(dir: &std::path::Path, net: &str, secs: u64, publish: bool) -> (Child, BufReader<std::process::ChildStdout>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_poolpeer"));
    cmd.arg(dir).arg(net).arg(secs.to_string());
    if publish {
        cmd.arg("publish");
    }
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::null()).spawn().expect("spawn poolpeer");
    let out = BufReader::new(child.stdout.take().unwrap());
    (child, out)
}

/// Read lines until `want` is true of one, or the deadline passes.
fn until(
    reader: &mut BufReader<std::process::ChildStdout>,
    within: Duration,
    mut want: impl FnMut(&str) -> bool,
) -> Option<String> {
    let deadline = Instant::now() + within;
    let mut line = String::new();
    while Instant::now() < deadline {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return None,
            Ok(_) => {
                if want(&line) {
                    return Some(line.clone());
                }
            }
            Err(_) => return None,
        }
    }
    None
}

fn field(line: &str, key: &str) -> i64 {
    line.split(&format!("\"{key}\":"))
        .nth(1)
        .and_then(|r| r.split(|c: char| !c.is_ascii_digit() && c != '-').find(|s| !s.is_empty()))
        .and_then(|n| n.parse().ok())
        .unwrap_or(-1)
}

/// TWO PROCESSES FIND EACH OTHER AT ALL.
///
/// The floor. If this fails, nothing below it means anything, and every
/// in-process test in this crate is measuring a function call.
#[test]
fn two_separate_processes_see_each_other_in_the_pool() {
    let net = "twoproc-see";
    let (mut a, mut ra) = peer(&tmp("see-a"), net, 40, true);
    let (mut b, mut rb) = peer(&tmp("see-b"), net, 40, false);

    let saw_a = until(&mut ra, Duration::from_secs(35), |l| field(l, "pool") >= 2).is_some();
    let saw_b = until(&mut rb, Duration::from_secs(35), |l| field(l, "pool") >= 2).is_some();

    let _ = a.kill();
    let _ = b.kill();
    assert!(saw_a && saw_b, "two processes never saw each other in the pool (a={saw_a} b={saw_b})");
}

/// AND GOSSIP ACTUALLY CROSSES THE PROCESS BOUNDARY.
///
/// **THIS IS THE ONE THAT WOULD HAVE CAUGHT IT.** `wanted` is populated from
/// `Holding` announcements received over gossip — the same mechanism that
/// carries live operations. On two phones it produces nothing: `holders: none`
/// and `received_live_operations: 0` in every session. In one process it is
/// free. Between two processes on one machine it is a real test.
#[test]
fn a_holding_announcement_crosses_a_process_boundary() {
    let net = "twoproc-gossip";
    let (mut a, mut ra) = peer(&tmp("gos-a"), net, 60, true);
    let (mut b, mut rb) = peer(&tmp("gos-b"), net, 60, false);

    // The publisher has to be up and holding before anything can be announced.
    let up = until(&mut ra, Duration::from_secs(20), |l| l.contains("\"event\":\"up\"")).is_some();

    // The listener must learn, from gossip alone, that there is something to carry.
    let learned = until(&mut rb, Duration::from_secs(45), |l| field(l, "wanted") >= 1);

    let _ = a.kill();
    let _ = b.kill();
    assert!(up, "the publisher never started");
    assert!(
        learned.is_some(),
        "no Holding announcement crossed between two processes in 45s — \
         gossip is the mechanism that also carries live operations, and on two \
         phones it delivers nothing"
    );
}

/// AND A PEER ACTUALLY ENDS UP HOLDING SOMETHING.
///
/// The pool's whole promise (D15): any holder serves identical bytes, so a
/// subject whose phone is asleep can still be read. A pool where nobody adopts
/// provides nothing at the moment it is needed, and until now that was only
/// ever asserted in-process with `adopt` called by hand.
#[test]
fn a_second_process_adopts_without_being_told_to() {
    let net = "twoproc-adopt";
    let (mut a, mut ra) = peer(&tmp("ad-a"), net, 75, true);
    let (mut b, mut rb) = peer(&tmp("ad-b"), net, 75, false);

    let up = until(&mut ra, Duration::from_secs(20), |l| l.contains("\"event\":\"up\"")).is_some();
    let adopted = until(&mut rb, Duration::from_secs(60), |l| field(l, "held") >= 1);

    let _ = a.kill();
    let _ = b.kill();
    assert!(up, "the publisher never started");
    assert!(
        adopted.is_some(),
        "no peer ended up holding anything in 60s — D15's redundancy is the \
         reason a sleeping subject is still readable, and a pool where nobody \
         adopts provides nothing"
    );
}
