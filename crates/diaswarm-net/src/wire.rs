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

use diaswarm_core::seal::grant_tag;
use diaswarm_core::vault::{hex, Identity};

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
    vault: PathBuf,
}

impl VaultServer {
    pub fn new(vault: impl Into<PathBuf>) -> Self {
        VaultServer { vault: vault.into() }
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
                .and_then(|req| answer(&self.vault, &req).ok())
                .unwrap_or_default();
            send.write_all(&reply).await.map_err(AcceptError::from_err)?;
            send.finish().map_err(AcceptError::from_err)?;
        }
    }
}

/// Start serving. The returned router runs until it is shut down.
pub async fn serve(vault: PathBuf, secret: SecretKey) -> Result<Router> {
    serve_with(vault, secret, false).await
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
pub async fn serve_with(vault: PathBuf, secret: SecretKey, local_only: bool) -> Result<Router> {
    let endpoint = if local_only {
        Endpoint::builder(presets::Minimal).secret_key(secret).bind().await?
    } else {
        Endpoint::builder(presets::N0).secret_key(secret).bind().await?
    };
    Ok(Router::builder(endpoint).accept(ALPN, VaultServer::new(vault)).spawn())
}

async fn ask(conn: &Connection, req: &Request) -> Result<Vec<u8>> {
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(&serde_json::to_vec(req)?).await?;
    send.finish()?;
    Ok(recv.read_to_end(64 * 1024 * 1024).await?)
}

/// Fetch a whole vault, plus the wraps for one tag, into a local directory.
///
/// Fetches EVERY segment, not only the ones this tag can open. Deliberate on
/// two counts: a peer holding ciphertext it cannot read is the property the
/// architecture rests on (§7.1), and fetching only what you can open would tell
/// anyone watching exactly what you were granted — which is the leak D13 just
/// closed in the grant log, reintroduced through traffic.
pub async fn fetch(addr: EndpointAddr, tag: &str, into: &Path) -> Result<(usize, usize)> {
    fetch_with(addr, Who::Tag(tag.to_string()), into, false).await
}

/// Who is fetching.
///
/// A reader knows their own secret but not, before the manifest arrives, the
/// subject's public key — and the tag needs both. Passing the identity lets the
/// tag be derived mid-flight, which is why this is not just a string.
///
/// The alternative, and what this replaced, was fetching the whole vault once
/// with a placeholder tag purely to learn the subject, then fetching it all
/// again. That downloaded six megabytes twice over a relay to answer a question
/// the manifest had already answered.
pub enum Who {
    /// Derive the tag once the subject is known.
    Reader { identity: Identity, purpose: String },
    /// A tag already computed.
    Tag(String),
}

pub async fn fetch_as(
    addr: EndpointAddr,
    who: Who,
    into: &Path,
    local_only: bool,
) -> Result<(usize, usize)> {
    fetch_with(addr, who, into, local_only).await
}

pub async fn fetch_with(
    addr: EndpointAddr,
    who: Who,
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
        serde_json::from_slice(&ask(&conn, &Request::Manifest).await?).context("manifest")?;
    let grants = ask(&conn, &Request::Grants).await?;
    install(into, &manifest, &grants)?;

    // The subject is in the manifest, so the tag can be derived now — one pass.
    let tag = match &who {
        Who::Tag(t) => t.clone(),
        Who::Reader { identity, purpose } => {
            let meta: serde_json::Value = serde_json::from_str(&manifest.meta)?;
            let subject_hex = meta["subject"].as_str().context("meta has no subject")?;
            let bytes = diaswarm_core::vault::unhex(subject_hex)
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            let subject: [u8; 32] =
                bytes.try_into().map_err(|_| anyhow::anyhow!("subject key is not 32 bytes"))?;
            hex(&grant_tag(&identity.encryption, &subject, purpose))
        }
    };

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
        let bytes = ask(&conn, &Request::Segment { seq: *seq, epoch: *epoch }).await?;
        install_segment(into, *seq, *epoch, &bytes)?;
        fetched += 1;
    }

    let wraps: Vec<WrapBlob> =
        serde_json::from_slice(&ask(&conn, &Request::Wraps { tag: tag.clone() }).await?)
            .unwrap_or_default();
    let opened = wraps.len();
    for w in wraps {
        install_wrap(into, w.seq, &tag, &w.bytes)?;
    }
    let _ = fetched;

    conn.close(0u32.into(), b"done");
    endpoint.close().await;
    Ok((fetched, opened))
}
