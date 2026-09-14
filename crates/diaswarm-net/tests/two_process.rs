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
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// **ONE AT A TIME, OR THE SUITE LIES ABOUT THE TRANSPORT.**
///
/// Each test here spawns two peers, and each peer runs an iroh endpoint, mDNS
/// and gossip. Four tests at once is eight of those on one machine competing
/// for discovery, and it does not fail cleanly — it fails a *different* test
/// each run. Measured: two consecutive parallel runs failed
/// `two_separate_processes_see_each_other_in_the_pool` and then
/// `a_holding_announcement_crosses_a_process_boundary`; the same suite run
/// serially passed 4/4 twice, in 22s and 26s.
///
/// A flaky transport suite is worse than none, because it teaches you to
/// discount a red result — and the whole reason this file exists is that a real
/// transport defect went unnoticed for the life of the project.
///
/// Serialising here rather than asking for `--test-threads=1` on the command
/// line: a guarantee nobody has to remember is the only kind that holds.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// Take the lock, ignoring poisoning — a panic in one test must not turn the
/// rest of the file into errors that hide the original failure.
fn alone() -> MutexGuard<'static, ()> {
    ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner())
}

fn tmp(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-2p-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// Spawn a keys peer — publisher or follower — in its own process.
fn keys_peer(
    dir: &std::path::Path,
    net: &str,
    secs: u64,
    role: &str,
    subject: Option<&str>,
) -> (Child, BufReader<std::process::ChildStdout>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_keyspeer"));
    cmd.arg(dir).arg(net).arg(secs.to_string()).arg(role);
    if let Some(s) = subject {
        cmd.arg(s);
    }
    let mut child =
        cmd.stdout(Stdio::piped()).stderr(Stdio::null()).spawn().expect("spawn keyspeer");
    let out = BufReader::new(child.stdout.take().unwrap());
    (child, out)
}

/// The `"key":"value"` string a JSON line carries, if it carries one.
fn text(line: &str, key: &str) -> Option<String> {
    line.split(&format!("\"{key}\":\"")).nth(1)?.split('"').next().map(|s| s.to_string())
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
    let _alone = alone();
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
    let _alone = alone();
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
    let _alone = alone();
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

/// AND A SEGMENT PUBLISHED NOW CROSSES A PROCESS BOUNDARY NOW.
///
/// **THE REGRESSION THIS SUITE EXISTS TO PREVENT, AT THE ONLY SCALE THAT
/// COUNTS.** `tests/live_mode.rs` asserts the same property with both peers
/// under one runtime, which proves `SyncHandle::publish` is called and that the
/// far end stores what it is handed. It does not prove that a *process* can
/// push to another process — and for the life of this project that was exactly
/// the thing that did not happen: 69 live-mode sessions across two phones, and
/// `received_live_operations: 0` in every one of them.
///
/// The follower's `live` count is the assertion. `received` climbing is not
/// enough: catch-up delivers everything, which is why the total was always
/// healthy while the transport was half-built.
#[test]
fn a_segment_published_in_one_process_is_pushed_to_another() {
    let _alone = alone();
    let net = "twoproc-live";
    let (mut a, mut ra) = keys_peer(&tmp("live-a"), net, 75, "publish", None);

    // The publisher names its own subject on the way up; the follower cannot
    // be started until it is known.
    let up = until(&mut ra, Duration::from_secs(25), |l| l.contains("\"event\":\"up\""));
    let subject = up.as_deref().and_then(|l| text(l, "subject"));
    let Some(subject) = subject else {
        let _ = a.kill();
        panic!("the publisher never came up");
    };

    let (mut b, mut rb) = keys_peer(&tmp("live-b"), net, 75, "follow", Some(&subject));

    // It has to have fetched the history first, or "arrived live" would be
    // unfalsifiable — everything arrives somehow, and the question is how.
    let caught_up = until(&mut rb, Duration::from_secs(45), |l| field(l, "received") >= 1).is_some();

    // The publisher seals and pushes a segment every second from t=10, and
    // reports the running total it has sent.
    let mut last_sent: Option<i64> = None;
    let pushed = until(&mut ra, Duration::from_secs(45), |l| {
        let s = field(l, "sent");
        if s >= 0 {
            last_sent = Some(s);
        }
        field(l, "pushed") >= 1
    })
    .is_some();

    // And this is the one that was zero for the life of the project.
    let live = until(&mut rb, Duration::from_secs(45), |l| field(l, "live") >= 1);

    let _ = a.kill();
    let _ = b.kill();
    assert!(caught_up, "the follower never received anything at all, live or otherwise");
    assert!(
        pushed,
        "the publisher never had a live stream to push onto — it carries its own \
         subject, so a 0 here means the handle was dropped or never kept"
    );
    assert!(
        live.is_some(),
        "nothing arrived over gossip in 45s. Catch-up worked and live mode did \
         not, which is the exact shape of the defect that made every follower \
         this project ever shipped one sync interval behind"
    );

    // **AND NOT MORE THAN WERE SENT, WHICH IS THE HARDER HALF.** A follower
    // cannot receive more pushes than a publisher published. Four versions of
    // the live counter over-reported, two of them impossibly, and each was
    // believed for a while because the only assertion was `>= 1` — which a
    // counter that fires on everything passes trivially. Measured on hardware
    // at its worst: a phone reporting 4,872 live arrivals out of 4,890
    // operations while its only publisher had pushed four times.
    let sent = last_sent.unwrap_or(0);
    let got = live.as_deref().map(|l| field(l, "live")).unwrap_or(0);
    assert!(
        sent > 0,
        "the publisher never recorded sending anything, so the ceiling below is not a ceiling"
    );
    assert!(
        got <= sent,
        "the follower counted {got} pushed arrivals from a publisher that pushed \
         {sent}. An over-reporting transport diagnostic is worse than none: it \
         says the thing works whether or not it does"
    );
}
