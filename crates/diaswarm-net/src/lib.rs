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
pub const ALPN: &[u8] = b"diaswarm/1";

/// One request. One per stream, answered with raw bytes then a clean close.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Request {
    /// What this vault holds: meta, and the segments that exist.
    Manifest,
    /// The signed grant log. Requested by everyone, readable by everyone,
    /// meaningful only to those who can compute a tag in it.
    Grants,
    /// One sealed segment.
    Segment { seq: u64, epoch: i64 },
    /// Every wrap filed under one tag.
    Wraps { tag: String },
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
}

/// One wrap, as it travels.
#[derive(Debug, Serialize, Deserialize)]
pub struct WrapBlob {
    pub seq: u64,
    pub bytes: Vec<u8>,
}

fn segments_dir(vault: &Path) -> PathBuf {
    vault.join("segments")
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
    Ok(Manifest { meta, segments })
}

/// Answer one request from the vault on disk.
///
/// **Reads only, and only from inside the vault.** `seq` and `epoch` are numbers
/// and `tag` is checked to be hex, so nothing a caller sends can become a path
/// component — a request is not a filename, and letting it be one would turn
/// every peer into a file server for the whole device.
pub fn answer(vault: &Path, req: &Request) -> Result<Vec<u8>> {
    match req {
        Request::Manifest => Ok(serde_json::to_vec(&manifest(vault)?)?),
        Request::Grants => Ok(std::fs::read(vault.join("grants.ndjson")).unwrap_or_default()),
        Request::Segment { seq, epoch } => {
            let path = segments_dir(vault).join(format!("{seq}.{epoch}.seal"));
            Ok(std::fs::read(path).context("no such segment")?)
        }
        Request::Wraps { tag } => {
            if tag.len() != 64 || !tag.bytes().all(|b| b.is_ascii_hexdigit()) {
                anyhow::bail!("a tag is 64 hex characters");
            }
            let mut out = Vec::new();
            let wraps = vault.join("wraps");
            if wraps.exists() {
                for seg in std::fs::read_dir(&wraps)? {
                    let seg = seg?;
                    let Ok(seq) = seg.file_name().to_string_lossy().parse::<u64>() else { continue };
                    let path = seg.path().join(format!("{tag}.wrap"));
                    if let Ok(bytes) = std::fs::read(path) {
                        out.push(WrapBlob { seq, bytes });
                    }
                }
            }
            out.sort_by_key(|w| w.seq);
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

pub fn install_wrap(into: &Path, seq: u64, tag: &str, bytes: &[u8]) -> Result<()> {
    let dir = into.join("wraps").join(seq.to_string());
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(format!("{tag}.wrap")), bytes)?;
    Ok(())
}
