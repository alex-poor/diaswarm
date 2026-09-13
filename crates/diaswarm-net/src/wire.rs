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


use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use crate::{answer, install, install_segment, install_wrap, Manifest, Request, WrapBlob};

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
    /// Epoch millis until which a `Request::Offer` is accepted; 0 for never.
    ///
    /// Shared with whoever owns the endpoint, so the UI can open the window
    /// when somebody puts their invite on screen and let it lapse on its own.
    /// A deadline rather than a flag: a window that has to be closed is a
    /// window somebody forgets to close.
    accepting_until: Arc<AtomicI64>,
}

impl VaultServer {
    pub fn new(store: impl Into<PathBuf>) -> Self {
        VaultServer { store: store.into(), accepting_until: Arc::new(AtomicI64::new(0)) }
    }

    /// The handle the UI holds to open the window. See [`Request::Offer`].
    pub fn window(&self) -> Arc<AtomicI64> {
        self.accepting_until.clone()
    }
}

/// Now, in epoch milliseconds.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl ProtocolHandler for VaultServer {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        loop {
            let Ok((mut send, mut recv)) = connection.accept_bi().await else { return Ok(()) };
            let asked = recv.read_to_end(64 * 1024).await.map_err(AcceptError::from_err)?;
            // A request this peer cannot service closes that stream and nothing
            // else. One peer asking nonsense must not drop a connection another
            // peer is using.
            // The caller is deliberately not looked at. Every peer gets the
            // same answer, and nothing on disk changes because somebody asked.
            let reply = match serde_json::from_slice::<Request>(&asked) {
                // THE ONLY REQUEST THAT WRITES, and only while invited to.
                Ok(Request::Offer { invite }) => {
                    let open = self.accepting_until.load(Ordering::Relaxed) > now_ms();
                    if open { take_offer(&self.store, &invite) } else { Vec::new() }
                }
                // **RECORDED WITHOUT BEING BELIEVED.** Verifying needs the
                // subject's encryption secret, which this handler is not given
                // and should not be: it answers strangers. The claim goes in a
                // capped queue and is checked where grants already happen.
                //
                // Unlike `Offer` there is no acceptance window, because there
                // is nothing to protect: a claim costs a line in a file and
                // buys nothing until it verifies against a reader this subject
                // has already granted.
                Ok(Request::Handover { tag, keys, proof }) => {
                    let claim = crate::peer::Handover { tag, keys, proof };
                    match crate::peer::record_handover(&self.store, claim) {
                        Ok(true) => b"1".to_vec(),
                        _ => Vec::new(),
                    }
                }
                Ok(req) => answer(&self.store, &req).unwrap_or_default(),
                Err(_) => Vec::new(),
            };
            send.write_all(&reply).await.map_err(AcceptError::from_err)?;
            send.finish().map_err(AcceptError::from_err)?;
        }
    }
}

/// Hand our invite to somebody who just showed us theirs.
///
/// Sent on the same endpoint and ALPN as a fetch, so a peer that shares and a
/// peer that reads are indistinguishable on the wire — the same property D15
/// gives relays. Returns whether they took it; a peer whose window has closed
/// replies empty, and that is not an error worth shouting about.
pub async fn offer_on(
    endpoint: &iroh::Endpoint,
    alpn: &[u8],
    addr: iroh::EndpointAddr,
    invite: &str,
) -> Result<bool> {
    let conn = endpoint.connect(addr, alpn).await?;
    let reply = ask(&conn, &Request::Offer { invite: invite.to_string() }).await?;
    conn.close(0u32.into(), b"done");
    Ok(!reply.is_empty())
}

/// Offer a keys identity to a subject that already granted us.
///
/// **THE READER'S HALF OF D27.** Returns whether it was taken, and a subject
/// too old to understand the request replies empty — which reads as "not
/// taken", which is exactly right: nothing that was working stops, the reader
/// goes on reading the vault it already reads.
pub async fn hand_over(
    endpoint: &iroh::Endpoint,
    alpn: &[u8],
    addr: iroh::EndpointAddr,
    tag: &str,
    keys: &str,
    proof: &str,
) -> Result<bool> {
    let conn = endpoint.connect(addr, alpn).await?;
    let reply = ask(
        &conn,
        &Request::Handover {
            tag: tag.to_string(),
            keys: keys.to_string(),
            proof: proof.to_string(),
        },
    )
    .await?;
    conn.close(0u32.into(), b"done");
    Ok(!reply.is_empty())
}

/// Record an invite somebody handed us, and say whose it was.
///
/// Deliberately quiet about failure: a malformed invite and a closed window
/// both produce an empty reply, because the sender is not owed a distinction
/// they could probe with. The person watching their own screen is.
fn take_offer(store: &Path, invite: &str) -> Vec<u8> {
    let Ok(inv) = diaswarm_core::invite::Invite::parse(invite) else { return Vec::new() };
    match crate::peer::add_follow_via(
        store,
        &inv.subject,
        &inv.endpoint,
        Some(&inv.purpose),
        Some(inv.relay.as_str()),
        // **THE SUBJECT'S KEYS BUNDLE, ARRIVING BY THE ROUTE THAT ALREADY
        // EXISTED.** `Offer` carries an invite, and an invite now carries a
        // bundle, so the reader's half of the D26 pairing needs no new request
        // and no ALPN bump — which is the whole reason for putting the bundle
        // in the invite rather than in a message of its own.
        (!inv.keys.is_empty()).then_some(inv.keys.as_str()),
    ) {
        Ok(_) => serde_json::to_vec(&inv.subject).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Hand an invite over on a throwaway endpoint. The peer-owned equivalent is
/// [`offer_on`], and a phone should use that.
pub async fn offer_with(addr: EndpointAddr, invite: &str, local_only: bool) -> Result<bool> {
    let endpoint = if local_only {
        Endpoint::bind(presets::Minimal).await?
    } else {
        Endpoint::bind(presets::N0).await?
    };
    // THE MIXED ALPN, ALWAYS, because that is what `serve_offering` accepts —
    // `local_only` chooses whether to use n0's discovery, not which dialect to
    // speak. Getting this wrong produces "peer doesn't support any known
    // protocol" from two peers that are otherwise perfectly configured, which
    // is the same mistake this file already warns about twice.
    let alpn = crate::swarm::default_wire_alpn();
    let out = offer_on(&endpoint, &alpn, addr, invite).await;
    endpoint.close().await;
    out
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
    Ok(serve_offering(store, secret, local_only).await?.0)
}

/// The same, handing back the offer window so a caller can open it.
///
/// Plain [`serve_with`] drops the handle, which means it never accepts a pushed
/// invite — the right default for something serving to whoever asks.
pub async fn serve_offering(
    store: PathBuf,
    secret: SecretKey,
    local_only: bool,
) -> Result<(Router, Arc<AtomicI64>)> {
    let endpoint = if local_only {
        Endpoint::builder(presets::Minimal).secret_key(secret).bind().await?
    } else {
        Endpoint::builder(presets::N0).secret_key(secret).bind().await?
    };
    // ON THE POOL'S ALPN, not the plain constant. Otherwise this crate speaks
    // two dialects: a peer that joined through `swarm.rs` answers on the mixed
    // id and a peer started by the CLI answers on the raw one, and neither can
    // reach the other while both look perfectly healthy.
    let server = VaultServer::new(store);
    let window = server.window();
    Ok((Router::builder(endpoint).accept(crate::swarm::default_wire_alpn(), server).spawn(), window))
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
    let conn = endpoint
        .connect(addr, &crate::swarm::default_wire_alpn()[..])
        .await
        .context("connect")?;
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
    // A THROWAWAY IDENTITY, and nothing may depend on it. The endpoint lives
    // for the length of one fetch; prefer `fetch_on`, which uses the peer's own
    // endpoint, wherever this peer has one.
    let conn = endpoint
        .connect(addr, &crate::swarm::default_wire_alpn()[..])
        .await
        .context("connect")?;
    let out = fetch_over(&conn, subject, into).await;
    conn.close(0u32.into(), b"done");
    endpoint.close().await;
    out
}

/// Fetch using an endpoint that already exists — this peer's own.
///
/// **A PEER HAS ONE IDENTITY, and this is what makes that true.** Fetching used
/// to bind a fresh endpoint every time, so the id the far side saw was a
/// throwaway that existed for the length of one sync. Everything still worked,
/// because nothing depended on it — until discovery did. Then a subject
/// faithfully recorded the address of a peer that had already ceased to exist,
/// and handed it to followers as somewhere to look.
///
/// Serving and fetching from the same endpoint means the id a peer is known by
/// is the id it can be reached at.
pub async fn fetch_on(
    endpoint: &Endpoint,
    addr: EndpointAddr,
    subject: &str,
    into: &Path,
) -> Result<(usize, usize)> {
    fetch_on_alpn(endpoint, addr, &crate::swarm::default_wire_alpn(), subject, into).await
}

/// The same, naming the ALPN to dial with.
///
/// **BECAUSE A POOLED PEER DOES NOT ANSWER ON `ALPN`.** p2panda mixes the
/// protocol id with its network id before handing it to iroh, so a peer inside
/// the pool listens on `Hash(ALPN ++ network_id)` and a direct dial carrying
/// the plain string is refused with "peer doesn't support any known protocol" —
/// which reads exactly like a peer that has gone away. See
/// [`crate::swarm::wire_alpn`].
pub async fn fetch_on_alpn(
    endpoint: &Endpoint,
    addr: EndpointAddr,
    alpn: &[u8],
    subject: &str,
    into: &Path,
) -> Result<(usize, usize)> {
    let conn = endpoint.connect(addr, alpn).await.context("connect")?;
    let out = fetch_over(&conn, subject, into).await;
    conn.close(0u32.into(), b"done");
    out
}

/// Fetch over a connection somebody else opened.
///
/// Split out so the pool can use it: p2panda dials by node id and hands back a
/// connection, and everything after that is the protocol this crate already
/// spoke. The wire did not need to change to join a swarm — only the way peers
/// find each other did.
pub async fn fetch_over(
    conn: &Connection,
    subject: &str,
    into: &Path,
) -> Result<(usize, usize)> {
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
