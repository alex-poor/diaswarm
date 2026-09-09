//! Moving a vault between peers, over iroh.
//!
//! WHY THIS NEEDS NO PRIVILEGED CHANNEL. Everything a vault holds is already
//! ciphertext or already public: segments are sealed, wraps are encrypted to one
//! reader, and the grant log names nobody (D13). So the protocol serves whatever
//! is asked for, to whoever asks, and the access control is the same as it is
//! everywhere else in this design — who holds a key, not who a server decides to
//! serve.
//!
//! That is not a shortcut. feasibility.md §7.1 says every member holds some of
//! everyone's ciphertext, and §9.2 that who *holds* the bytes should be as many
//! peers as possible while who can *read* them is decided entirely by key
//! distribution. A transport that authenticated readers would be re-introducing
//! the thing the architecture removed.
//!
//! WHAT REPLICATES, and the third item is a requirement rather than a
//! convenience (D13):
//!
//!   * **sealed segments** — the history, unreadable without a wrap;
//!   * **wraps** — one per reader per segment, useless to anyone else;
//!   * **the grant log** — because tamper-evidence against the subject rests on
//!     other copies existing. A subject who can truncate the only copy has a log
//!     that proves nothing.
//!
//! WHAT THIS DOES NOT DO. It is pull, not gossip: a reader asks, a subject
//! answers. That is enough to demonstrate the property across a real network and
//! is the wrong shape for the flagship, where a follower needs data while the
//! subject's phone is asleep. Push and store-and-forward are the next problem,
//! not this one.

pub mod peer;
pub mod wire;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// The protocol version, negotiated by QUIC before a byte is exchanged.
///
/// **Bumped whenever the wire shape changes.** Adding a size to the manifest
/// was a silent break: peers still connected, and the mismatch surfaced as a
/// failed request that a follower reported as "unreachable" — the subject
/// looked offline when it was merely older. An ALPN mismatch refuses the
/// connection instead, which is a thing a person can act on.
pub const ALPN: &[u8] = b"diaswarm/4";

/// One request. One per stream, answered with raw bytes then a clean close.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Request {
    /// Which subjects this peer holds anything for.
    ///
    /// The question that turns a set of nodes into a swarm: a reader asks
    /// whoever it can reach, rather than only the subject whose data it wants.
    Have,
    /// What this peer holds for one subject: meta, and the segments.
    Manifest { subject: String },
    /// The signed grant log. Requested by everyone, readable by everyone,
    /// meaningful only to those who can compute a tag in it.
    Grants { subject: String },
    /// One sealed segment.
    Segment { subject: String, seq: u64, epoch: i64 },
    /// "I am a peer, I hold this subject, and here is where to reach me."
    ///
    /// The ADDRESSES are claimed; the identity is not. A peer's id comes from
    /// the authenticated QUIC connection, so this can only ever describe the
    /// sender — it cannot announce somebody else, which is what would make
    /// discovery a way to aim followers at an address of an attacker's
    /// choosing. A bogus address costs one failed dial, because reaching it
    /// still requires a handshake matching the announced id.
    ///
    /// Needed because a bare endpoint id can only be resolved through a
    /// discovery service. Carrying addresses is what lets two phones on the
    /// same wifi find each other with the uplink down.
    Announce { subject: String, addrs: Vec<String> },
    /// Which other peers this one knows hold a subject.
    ///
    /// THE ANSWER TO "AVAILABLE WHEN YOUR PHONE IS OFF". Without it a follower
    /// knows exactly one address — the one it scanned — so the moment that
    /// device sleeps there is nowhere else to ask, and the whole point of
    /// replicating to a swarm goes unrealised. The relay case was demonstrable
    /// only because somebody typed a second address in by hand.
    ///
    /// **This publishes the follower set**, and that is a real cost, not a
    /// footnote: anyone holding a subject's public key can ask any peer who
    /// else carries that subject, and get back a list of stable, dialable
    /// endpoint ids. It is the social-graph leak feasibility.md §9 already
    /// names, arriving through the door that makes availability work. See D18.
    Holders { subject: String },
    /// EVERY wrap this peer holds for a subject, whoever they are for.
    ///
    /// Asking for one tag was the obvious design and it was wrong twice over.
    ///
    /// It made relaying impossible: a peer holding a subject it cannot read
    /// does not know anyone else's tags, so it replicated segments and no
    /// wraps — a copy of the history that opens for nobody, which is not a
    /// replica of anything. The swarm demo only worked because the relaying
    /// laptop happened to be fetching AS the reader.
    ///
    /// And it leaked: naming your tag tells the peer you are dialling exactly
    /// which entry in the public grant log you are. D13 took the reader's name
    /// out of the log; asking for it by name over the wire put it back. Now
    /// every fetcher sends the same request, so the traffic says nothing.
    ///
    /// Costs about 100 bytes per reader per segment — 15 KB for a year of one
    /// reader, and the manifest's count means it is skipped when nothing has
    /// changed.
    Wraps { subject: String },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub meta: String,
    /// `(seq, epoch, bytes)` for each segment held.
    ///
    /// THE SIZE IS WHAT MAKES SYNC INCREMENTAL, and getting this wrong cost a
    /// follower its freshness. The first attempt assumed only the newest
    /// segment could grow, so it re-fetched that one and skipped the rest. But
    /// there is an open segment PER EPOCH — `open.json` maps each one — and the
    /// segment still being written for today is usually not the highest seq. A
    /// follower sat there reporting a reading that aged and never changed,
    /// which is precisely the failure the age display exists to catch.
    ///
    /// Comparing sizes needs no such assumption: re-fetch what differs.
    pub segments: Vec<(u64, i64, u64)>,
    /// How many wrap files this peer holds, across every reader.
    ///
    /// A wrap is never re-issued, so the count only grows: equal counts mean
    /// there is nothing to fetch. That is the whole of the incremental check,
    /// and it is enough because these are immutable.
    #[serde(default)]
    pub wraps: usize,
}

/// One wrap, as it travels.
#[derive(Debug, Serialize, Deserialize)]
pub struct WrapBlob {
    pub seq: u64,
    /// Which reader it is for — unlinkable to a person without their key.
    pub tag: String,
    pub bytes: Vec<u8>,
}

fn segments_dir(vault: &Path) -> PathBuf {
    vault.join("segments")
}

/// How many peers are remembered per subject.
///
/// A bound rather than a policy: an unbounded list is a place for anyone to
/// write as much as they like into somebody else's storage, and a follower only
/// needs a handful of addresses to survive one device sleeping.
const MAX_HOLDERS: usize = 32;

fn holders_path(vault: &Path) -> PathBuf {
    vault.join("holders.json")
}

/// Peers known to hold this subject.
pub fn holders(vault: &Path) -> Vec<String> {
    std::fs::read(holders_path(vault))
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
        .unwrap_or_default()
}

/// Remember that a peer holds, or is about to hold, this subject.
///
/// Called when somebody asks for the manifest, because asking for it is what
/// you do immediately before replicating — so the set of people who have asked
/// is a good approximation of the set of people who have a copy.
///
/// Nothing here is taken on trust: the id comes from the authenticated QUIC
/// connection, so a peer can only ever add ITSELF to somebody's list. It cannot
/// name a third party, which is what would make this a way to point followers
/// at an address of an attacker's choosing.
pub fn remember_holder(vault: &Path, endpoint: &str) -> Result<()> {
    // Either a bare endpoint id, or `id@addr` — the id is what authenticates,
    // the address is only a hint for reaching it without discovery.
    let id = endpoint.split('@').next().unwrap_or("");
    if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(());
    }
    let mut known = holders(vault);
    match known.iter().position(|h| h.split('@').next() == Some(id)) {
        // Already known. Keep whichever form says more: an entry that carries
        // an address can be dialled on a network with no discovery at all,
        // which is exactly when a local swarm matters most.
        Some(at) => {
            if endpoint.len() > known[at].len() {
                known[at] = endpoint.to_string();
                std::fs::create_dir_all(vault)?;
                std::fs::write(holders_path(vault), serde_json::to_vec(&known)?)?;
            }
            return Ok(());
        }
        None => {}
    }
    known.push(endpoint.to_string());
    // Oldest out first. A peer that has gone for good should not keep a slot
    // forever just because it was early.
    while known.len() > MAX_HOLDERS {
        known.remove(0);
    }
    std::fs::create_dir_all(vault)?;
    std::fs::write(holders_path(vault), serde_json::to_vec(&known)?)?;
    Ok(())
}

/// Turn a requested subject into a path inside the store.
///
/// **Checked, not trusted.** A subject is 64 hex characters and nothing else,
/// so a request cannot become a path component and a peer cannot be turned into
/// a file server for the whole device. This is the only place a caller's bytes
/// reach the filesystem.
fn vault_of(store: &Path, subject: &str) -> Result<PathBuf> {
    if subject.len() != 64 || !subject.bytes().all(|b| b.is_ascii_hexdigit()) {
        anyhow::bail!("a subject is 64 hex characters");
    }
    Ok(store.join(subject.to_ascii_lowercase()))
}

/// Read the manifest straight off disk.
pub fn manifest(vault: &Path) -> Result<Manifest> {
    let meta = std::fs::read_to_string(vault.join("meta.json")).context("meta.json")?;
    let mut segments = Vec::new();
    for entry in std::fs::read_dir(segments_dir(vault))? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(stem) = name.strip_suffix(".seal") else { continue };
        let mut parts = stem.splitn(2, '.');
        if let (Some(a), Some(b)) = (parts.next(), parts.next()) {
            if let (Ok(seq), Ok(epoch)) = (a.parse::<u64>(), b.parse::<i64>()) {
                let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
                segments.push((seq, epoch, len));
            }
        }
    }
    segments.sort_unstable();
    Ok(Manifest { meta, segments, wraps: count_wraps(vault) })
}

/// How many wrap files a vault holds, across every reader.
fn count_wraps(vault: &Path) -> usize {
    let wraps = vault.join("wraps");
    let Ok(dirs) = std::fs::read_dir(&wraps) else { return 0 };
    let mut n = 0;
    for seg in dirs.flatten() {
        if let Ok(files) = std::fs::read_dir(seg.path()) {
            n += files.flatten().filter(|f| f.file_name().to_string_lossy().ends_with(".wrap")).count();
        }
    }
    n
}

/// Answer one request from the vault on disk.
///
/// **Reads only, and only from inside the vault.** `seq` and `epoch` are numbers
/// and `tag` is checked to be hex, so nothing a caller sends can become a path
/// component — a request is not a filename, and letting it be one would turn
/// every peer into a file server for the whole device.
pub fn answer(store: &Path, req: &Request) -> Result<Vec<u8>> {
    answer_from(store, req, None)
}

/// Answer, knowing who is asking.
///
/// The caller's id is the authenticated endpoint on the other end of the QUIC
/// connection, not anything it claimed. It is used for one thing: noticing that
/// this peer is about to hold a copy of the subject, so that a later follower
/// can be told where else to look.
pub fn answer_from(store: &Path, req: &Request, caller: Option<&str>) -> Result<Vec<u8>> {
    match req {
        Request::Have => {
            let mut subjects: Vec<String> = Vec::new();
            if store.exists() {
                for entry in std::fs::read_dir(store)? {
                    let entry = entry?;
                    if entry.path().join("meta.json").exists() {
                        subjects.push(entry.file_name().to_string_lossy().to_string());
                    }
                }
            }
            subjects.sort();
            Ok(serde_json::to_vec(&subjects)?)
        }
        Request::Manifest { subject } => {
            let vault = vault_of(store, subject)?;
            // Asking for the manifest is the step before replicating, so this
            // is where a peer learns that somebody else is about to have a copy.
            if let Some(id) = caller {
                let _ = remember_holder(&vault, id);
            }
            Ok(serde_json::to_vec(&manifest(&vault)?)?)
        }
        Request::Announce { subject, addrs } => {
            let vault = vault_of(store, subject)?;
            if let Some(id) = caller {
                let clean: Vec<&str> = addrs
                    .iter()
                    .map(String::as_str)
                    .filter(|a| a.parse::<std::net::SocketAddr>().is_ok())
                    .take(8)
                    .collect();
                let entry =
                    if clean.is_empty() { id.to_string() } else { format!("{id}@{}", clean.join(",")) };
                let _ = remember_holder(&vault, &entry);
            }
            Ok(Vec::new())
        }
        Request::Holders { subject } => {
            let mut known = holders(&vault_of(store, subject)?);
            // Telling someone about themselves is noise, and would make a
            // follower dial itself.
            if let Some(id) = caller {
                known.retain(|h| h != id);
            }
            Ok(serde_json::to_vec(&known)?)
        }
        Request::Grants { subject } => Ok(std::fs::read(
            vault_of(store, subject)?.join("grants.ndjson"),
        )
        .unwrap_or_default()),
        Request::Segment { subject, seq, epoch } => {
            let vault = vault_of(store, subject)?;
            let path = segments_dir(&vault).join(format!("{seq}.{epoch}.seal"));
            Ok(std::fs::read(path).context("no such segment")?)
        }
        Request::Wraps { subject } => {
            let vault = vault_of(store, subject)?;
            let mut out = Vec::new();
            let wraps = vault.join("wraps");
            if wraps.exists() {
                for seg in std::fs::read_dir(&wraps)? {
                    let seg = seg?;
                    let Ok(seq) = seg.file_name().to_string_lossy().parse::<u64>() else { continue };
                    for file in std::fs::read_dir(seg.path())?.flatten() {
                        let name = file.file_name().to_string_lossy().to_string();
                        let Some(tag) = name.strip_suffix(".wrap") else { continue };
                        // Names come off this peer's own disk, but they end up
                        // in a path on the receiver's, so they are checked
                        // here as well as there.
                        if tag.len() != 64 || !tag.bytes().all(|b| b.is_ascii_hexdigit()) {
                            continue;
                        }
                        if let Ok(bytes) = std::fs::read(file.path()) {
                            out.push(WrapBlob { seq, tag: tag.to_string(), bytes });
                        }
                    }
                }
            }
            out.sort_by(|a, b| (a.seq, &a.tag).cmp(&(b.seq, &b.tag)));
            Ok(serde_json::to_vec(&out)?)
        }
    }
}

/// Write a fetched vault to disk, ready for `diaswarm read`.
pub fn install(into: &Path, manifest: &Manifest, grants: &[u8]) -> Result<()> {
    std::fs::create_dir_all(segments_dir(into))?;
    std::fs::create_dir_all(into.join("wraps"))?;
    std::fs::write(into.join("meta.json"), &manifest.meta)?;
    if !grants.is_empty() {
        std::fs::write(into.join("grants.ndjson"), grants)?;
    }
    Ok(())
}

pub fn install_segment(into: &Path, seq: u64, epoch: i64, bytes: &[u8]) -> Result<()> {
    std::fs::write(segments_dir(into).join(format!("{seq}.{epoch}.seal")), bytes)?;
    Ok(())
}

/// Merge addresses learned from a peer into this replica's own list.
///
/// Merged rather than replaced: two peers each know a different corner of the
/// swarm, and taking only the last answer would make a follower forget the
/// address it had been using.
pub fn install_holders(into: &Path, learned: &[String]) -> Result<()> {
    for h in learned {
        remember_holder(into, h)?;
    }
    Ok(())
}

pub fn install_wrap(into: &Path, seq: u64, tag: &str, bytes: &[u8]) -> Result<()> {
    // The tag arrives from another peer and becomes a filename, so it is
    // checked here rather than trusted. `seq` is already a number.
    if tag.len() != 64 || !tag.bytes().all(|b| b.is_ascii_hexdigit()) {
        anyhow::bail!("a tag is 64 hex characters");
    }
    let dir = into.join("wraps").join(seq.to_string());
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(format!("{tag}.wrap")), bytes)?;
    Ok(())
}
