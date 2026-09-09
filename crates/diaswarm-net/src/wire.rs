//! The iroh side: serve a vault, fetch a vault.
//!
//! One request per bidirectional stream, answered with raw bytes and a clean
//! finish. No length prefixes and no framing of its own — the stream *is* the
//! frame, which is the whole reason to be on QUIC rather than to invent
//! something.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use iroh::endpoint::{presets, Connection};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, SecretKey};


use crate::{answer, install, install_segment, install_wrap, Manifest, Request, WrapBlob, ALPN};

/// Serves one vault to whoever asks.
///
/// **To whoever asks, deliberately.** A vault is sealed segments, wraps
/// encrypted to one reader, and a grant log that names nobody (D13) — there is
/// nothing here to withhold. Authenticating readers would re-introduce exactly
/// what feasibility.md §9.2 removed: access decided by who a server serves
/// rather than by who holds a key.
#[derive(Debug, Clone)]
pub struct VaultServer {
    store: PathBuf,
}

impl VaultServer {
    pub fn new(store: impl Into<PathBuf>) -> Self {
        VaultServer { store: store.into() }
    }
}

impl ProtocolHandler for VaultServer {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        loop {
            let Ok((mut send, mut recv)) = connection.accept_bi().await else { return Ok(()) };
            let asked = recv.read_to_end(64 * 1024).await.map_err(AcceptError::from_err)?;
            // A request this peer cannot service closes that stream and nothing
            // else. One peer asking nonsense must not drop a connection another
            // peer is using.
            let reply = serde_json::from_slice::<Request>(&asked)
                .ok()
                .and_then(|req| answer(&self.store, &req).ok())
                .unwrap_or_default();
            send.write_all(&reply).await.map_err(AcceptError::from_err)?;
            send.finish().map_err(AcceptError::from_err)?;
        }
    }
}

/// Start serving. The returned router runs until it is shut down.
pub async fn serve(store: PathBuf, secret: SecretKey) -> Result<Router> {
    serve_with(store, secret, false).await
}

/// Serve, optionally without n0's discovery and relays.
///
/// `local_only` exists for tests: the default preset reaches out to n0's
/// discovery service and relay network, so a test using it would be measuring
/// somebody else's uptime. Two endpoints on one machine find each other by
/// direct address without any of that.
///
/// It is not a privacy setting. feasibility.md §9.4 is explicit that NAT
/// traversal needs a relay and that this is unavoidable — a relay forwards
/// ciphertext and stores nothing, which is a different thing from a server.
pub async fn serve_with(store: PathBuf, secret: SecretKey, local_only: bool) -> Result<Router> {
    let endpoint = if local_only {
        Endpoint::builder(presets::Minimal).secret_key(secret).bind().await?
    } else {
        Endpoint::builder(presets::N0).secret_key(secret).bind().await?
    };
    Ok(Router::builder(endpoint).accept(ALPN, VaultServer::new(store)).spawn())
}

async fn ask(conn: &Connection, req: &Request) -> Result<Vec<u8>> {
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(&serde_json::to_vec(req)?).await?;
    send.finish()?;
    Ok(recv.read_to_end(64 * 1024 * 1024).await?)
}

/// Fetch a whole vault into a local directory.
///
/// Fetches EVERY segment and EVERY wrap, not only what the caller can open.
/// Deliberate on three counts: a peer holding ciphertext it cannot read is the
/// property the architecture rests on (§7.1); taking only what you can open
/// would tell anyone watching exactly what you were granted, which is the leak
/// D13 closed in the grant log arriving instead through traffic; and a copy
/// missing other readers' wraps cannot be passed on, so relaying would be
/// impossible and every reader would be back to depending on the subject.
///
/// Note what is NOT a parameter: who you are. A fetch is the same request for
/// everybody.
pub async fn fetch(addr: EndpointAddr, subject: &str, into: &Path) -> Result<(usize, usize)> {
    fetch_with(addr, subject, into, false).await
}

/// Which subjects a peer holds. The swarm question.
pub async fn have(addr: EndpointAddr, local_only: bool) -> Result<Vec<String>> {
    let endpoint = if local_only {
        Endpoint::bind(presets::Minimal).await?
    } else {
        Endpoint::bind(presets::N0).await?
    };
    let conn = endpoint.connect(addr, ALPN).await.context("connect")?;
    let subjects: Vec<String> = serde_json::from_slice(&ask(&conn, &Request::Have).await?)?;
    conn.close(0u32.into(), b"done");
    endpoint.close().await;
    Ok(subjects)
}

pub async fn fetch_as(
    addr: EndpointAddr,
    subject: &str,
    into: &Path,
    local_only: bool,
) -> Result<(usize, usize)> {
    fetch_with(addr, subject, into, local_only).await
}

/// Fetch one subject's vault from any peer that holds it.
///
/// **From any peer**, which is the whole point. The bytes are the same
/// wherever they come from — sealed segments and wraps nobody in between can
/// read — so availability stops depending on the subject's own phone being
/// awake.
pub async fn fetch_with(
    addr: EndpointAddr,
    subject: &str,
    into: &Path,
    local_only: bool,
) -> Result<(usize, usize)> {
    let endpoint = if local_only {
        Endpoint::bind(presets::Minimal).await?
    } else {
        Endpoint::bind(presets::N0).await?
    };
    let conn = endpoint.connect(addr, ALPN).await.context("connect")?;

    let manifest: Manifest =
        serde_json::from_slice(&ask(&conn, &Request::Manifest { subject: subject.to_string() }).await?).context("manifest")?;
    let grants = ask(&conn, &Request::Grants { subject: subject.to_string() }).await?;
    install(into, &manifest, &grants)?;

    // INCREMENTAL, BY SIZE. A follower syncs every few minutes and almost
    // nothing has changed; re-fetching the whole history to learn that would
    // move megabytes over a phone's connection.
    //
    // By size rather than by "the newest one": there is an open segment per
    // epoch, so the segment still being written today is usually not the
    // highest seq. Assuming otherwise left a follower re-fetching a segment
    // nothing was writing to, reporting a reading that aged and never changed.
    let mut fetched = 0usize;
    for (seq, epoch, len) in &manifest.segments {
        let path = into.join("segments").join(format!("{seq}.{epoch}.seal"));
        if path.metadata().map(|m| m.len()) .ok() == Some(*len) {
            continue;
        }
        let bytes = ask(&conn, &Request::Segment { subject: subject.to_string(), seq: *seq, epoch: *epoch }).await?;
        install_segment(into, *seq, *epoch, &bytes)?;
        fetched += 1;
    }

    // Wraps are immutable — one is never re-issued — so equal counts mean
    // there is nothing to ask for. Cheap, and it keeps a two-minute refresh
    // loop from moving every reader's wraps over and over.
    let held = count_local_wraps(into);
    let mut installed = 0usize;
    if held < manifest.wraps {
        // NOT `unwrap_or_default()`. A peer that fails to answer this replies
        // with nothing, and nothing is not valid JSON — so swallowing the parse
        // error reported "0 wraps" for a request the other end had refused.
        // That is the ALPN mistake again in a different place: a broken
        // exchange presented as a true and boring answer. An empty list is `[]`
        // and still parses, so the two states stay distinguishable.
        let reply = ask(&conn, &Request::Wraps { subject: subject.to_string() }).await?;
        let wraps: Vec<WrapBlob> = serde_json::from_slice(&reply)
            .with_context(|| format!("the peer did not answer for wraps ({} bytes)", reply.len()))?;
        for w in wraps {
            install_wrap(into, w.seq, &w.tag, &w.bytes)?;
            installed += 1;
        }
    }

    conn.close(0u32.into(), b"done");
    endpoint.close().await;
    Ok((fetched, installed))
}

/// How many wrap files are already on disk here.
fn count_local_wraps(vault: &Path) -> usize {
    let Ok(dirs) = std::fs::read_dir(vault.join("wraps")) else { return 0 };
    let mut n = 0;
    for seg in dirs.flatten() {
        if let Ok(files) = std::fs::read_dir(seg.path()) {
            n += files
                .flatten()
                .filter(|f| f.file_name().to_string_lossy().ends_with(".wrap"))
                .count();
        }
    }
    n
}
