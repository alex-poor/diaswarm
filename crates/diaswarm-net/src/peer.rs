//! A peer: holds what it follows, and serves what it holds.
//!
//! WHY THIS IS THE DIFFERENCE BETWEEN A SWARM AND A DEMO. Until this existed,
//! `follow` was a leaf — it took a subject's data and gave nothing back, so
//! every reader depended on the subject's own phone being awake. The swarm
//! property was real but staged: it worked once because a laptop was told, by
//! hand, to fetch a vault and then told, by hand, to serve it.
//!
//! A peer does both on its own. It serves its whole store (§9.2: who holds the
//! bytes should be as many peers as possible, while who can read them is
//! decided by keys) and refreshes what it follows on a timer. Two consequences
//! fall straight out:
//!
//!   * **Availability stops depending on one device.** A partner sees glucose
//!     while the subject's phone is asleep, because other followers hold it.
//!   * **The grant log replicates**, which D13 requires: tamper-evidence
//!     against the subject rests on other copies existing, and a subject who
//!     can truncate the only copy has a log that proves nothing.
//!
//! WHAT A PEER HOLDS IT USUALLY CANNOT READ. Following to relay is a
//! first-class case, not a degraded one. A relay stores segments, wraps it has
//! no key for, and the grant log — and is exactly as useful to the swarm as a
//! reader is, while learning nothing.
//!
//! MEASURED COST: a full year of one person's looping is about 25 MB. Holding
//! ten people is 250 MB/year, which is what makes "everyone holds everyone's
//! ciphertext" an affordable sentence rather than an aspirational one.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use diaswarm_core::vault::{hex, Identity};
use iroh::{EndpointAddr, EndpointId};

use crate::wire::{fetch_on_alpn, fetch_with};
use iroh::Endpoint;

/// Turn an upstream string into something dialable.
///
/// Two forms, and the second is not a convenience:
///
///   `<endpoint-id>`                       found through discovery
///   `<endpoint-id>@<ip:port>[,<ip:port>]` dialled directly
///
/// **A BARE ID NEEDS THE INTERNET.** Resolving one goes through n0's DNS and
/// pkarr services, so a bare id is unusable exactly when a local swarm would
/// be most useful: two phones on the same wifi with the uplink down, which for
/// this project is not a hypothetical — it is a hospital, a plane, or a flat
/// whose router has died. Direct addresses need nothing but the LAN.
///
/// The id is still what authenticates: it is a public key, and QUIC will not
/// complete a handshake with anyone else, whatever address answers.
pub fn parse_upstream(s: &str) -> Result<EndpointAddr> {
    let (id, rest) = match s.split_once('@') {
        Some((id, rest)) => (id, Some(rest)),
        None => (s, None),
    };
    let eid: EndpointId = id.parse().with_context(|| format!("not an endpoint id: {id}"))?;
    let mut addr = EndpointAddr::new(eid);
    for a in rest.into_iter().flat_map(|r| r.split(',')).filter(|a| !a.is_empty()) {
        let sock: std::net::SocketAddr =
            a.parse().with_context(|| format!("not an address: {a}"))?;
        addr = addr.with_ip_addr(sock);
    }
    Ok(addr)
}

/// One subject this peer keeps a copy of.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Follow {
    /// The subject's public key, in hex. Names the vault in the store.
    pub subject: String,
    /// Endpoints to try, in order.
    ///
    /// **A LIST, AND THAT IS THE WHOLE POINT.** One upstream is the failure
    /// this module exists to remove: if the only address you have is the
    /// subject's phone, then a peer that holds their history is useless the
    /// moment that phone sleeps. Any peer holding the subject serves the same
    /// bytes, so trying the next one is not a fallback — it is the design.
    pub from: Vec<String>,
    /// The purpose to try reading this under locally. `None` means relay only.
    ///
    /// It has nothing to do with what gets fetched — a fetch is the same
    /// request for everybody, and this peer receives every reader's wraps
    /// either way. It says only what this peer expects to be able to OPEN, so
    /// that "no wraps I can use" can be reported as a fault for a reader and
    /// as normal for a relay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    /// Where this subject can be reached when they are not on the same wifi,
    /// as it arrived in their invite (D23).
    ///
    /// **PER SUBJECT, NOT PER PHONE.** Two people you follow may be reachable
    /// through different relays — one on a public one, one on a relay their
    /// family runs — and a single global setting would make following both
    /// impossible. `None` is an invite from before relays existed, which means
    /// "however this build reaches people by default".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay: Option<String>,
    /// The subject's `diaswarm-keys` identity, as it arrived in their invite:
    /// the key that authors their logs, and the bundle a grant is agreed
    /// against.
    ///
    /// **KEPT BECAUSE JOINING HAPPENS LATER THAN PAIRING.** A reader needs this
    /// to find the subject's logs at all, to derive the tag it was granted
    /// under, and to open its welcome — and the welcome may not have replicated
    /// yet when the invite is scanned, or for hours if the subject's phone is
    /// asleep. Throwing it away at pairing time would mean the one moment it is
    /// available is the one moment it is not needed.
    ///
    /// `None` is a v1 or v2 invite: a subject with no keys vault, or one from
    /// before the field existed. Nothing about the old vault needs it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<String>,
}

impl Follow {
    /// Whether this peer expects to be able to read what it holds.
    ///
    /// Distinguishing these matters for what gets reported: a reader with no
    /// wraps has a problem worth shouting about, and a relay with no wraps is
    /// working exactly as intended. Conflating them would either hide a real
    /// fault or cry wolf at every relay.
    pub fn is_relay(&self) -> bool {
        self.purpose.is_none()
    }
}

/// A keys identity somebody has claimed, waiting to be checked.
///
/// **WRITTEN BY A NETWORK HANDLER, BELIEVED BY NOBODY.** Anyone who can reach
/// this peer can add one of these; that is why nothing here is trusted and why
/// the file is capped. Verification needs the subject's encryption secret, and
/// granting needs the keys vault, so both happen where those already are —
/// `Vault::accept_handover` and then a grant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Handover {
    /// The relationship tag the claimant says is theirs.
    pub tag: String,
    /// Their keys identity, hex.
    pub keys: String,
    /// `seal::handover_proof`, hex.
    pub proof: String,
}

/// Where claims wait. Beside `follows.json`, for the reason given there.
pub fn handovers_path(store: &Path) -> PathBuf {
    store.join("handovers.json")
}

/// **A CAP, BECAUSE THE WRITER IS A STRANGER.** Without one, anybody who can
/// reach this peer can grow a file on it for ever. Fifty is far more than a
/// person has followers and small enough that a flood costs nothing; the
/// oldest are dropped, because a real claim is retried and a flood is not
/// worth remembering.
const MAX_HANDOVERS: usize = 50;

pub fn load_handovers(store: &Path) -> Result<Vec<Handover>> {
    let path = handovers_path(store);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read(&path).with_context(|| format!("{}", path.display()))?;
    Ok(serde_json::from_slice(&raw).unwrap_or_default())
}

pub fn save_handovers(store: &Path, claims: &[Handover]) -> Result<()> {
    std::fs::create_dir_all(store)?;
    std::fs::write(handovers_path(store), serde_json::to_vec_pretty(claims)?)?;
    Ok(())
}

/// Record a claim. Returns whether anything changed.
///
/// Idempotent on the whole claim: a reader that retries because the subject was
/// asleep must not fill the queue with copies of itself.
pub fn record_handover(store: &Path, claim: Handover) -> Result<bool> {
    if claim.tag.is_empty() || claim.keys.is_empty() || claim.proof.is_empty() {
        return Ok(false);
    }
    let mut claims = load_handovers(store)?;
    if claims.contains(&claim) {
        return Ok(false);
    }
    claims.push(claim);
    if claims.len() > MAX_HANDOVERS {
        let drop = claims.len() - MAX_HANDOVERS;
        claims.drain(..drop);
    }
    save_handovers(store, &claims)?;
    Ok(true)
}

/// Take the queue, leaving it empty.
///
/// **TAKEN RATHER THAN READ**, so a claim that cannot be verified is dropped
/// instead of being retried on every pass for ever. A genuine reader whose
/// claim arrived while the subject was mid-rotation will send it again; a
/// forged one will not get another free attempt.
pub fn take_handovers(store: &Path) -> Result<Vec<Handover>> {
    let claims = load_handovers(store)?;
    if !claims.is_empty() {
        save_handovers(store, &[])?;
    }
    Ok(claims)
}

/// The peer's list of what to keep, stored beside the vaults it keeps.
///
/// In the store root rather than anywhere else, because a store is the unit
/// you would copy to a new machine, and a replica set that did not travel with
/// it would silently become a set of stale directories nothing refreshes.
pub fn follows_path(store: &Path) -> PathBuf {
    store.join("follows.json")
}

pub fn load_follows(store: &Path) -> Result<Vec<Follow>> {
    let path = follows_path(store);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read(&path).with_context(|| format!("{}", path.display()))?;
    serde_json::from_slice(&raw).with_context(|| format!("{} is not valid JSON", path.display()))
}

pub fn save_follows(store: &Path, follows: &[Follow]) -> Result<()> {
    std::fs::create_dir_all(store)?;
    std::fs::write(follows_path(store), serde_json::to_vec_pretty(follows)?)?;
    Ok(())
}

/// Add a subject to what this peer keeps, or add an upstream to an existing
/// one. Returns true if anything changed.
pub fn add_follow(store: &Path, subject: &str, from: &str, purpose: Option<&str>) -> Result<bool> {
    add_follow_via(store, subject, from, purpose, None, None)
}

/// The same, recording which relay this subject said to reach them through.
#[allow(clippy::too_many_arguments)]
pub fn add_follow_via(
    store: &Path,
    subject: &str,
    from: &str,
    purpose: Option<&str>,
    relay: Option<&str>,
    keys: Option<&str>,
) -> Result<bool> {
    let subject = subject.to_ascii_lowercase();
    if subject.len() != 64 || !subject.bytes().all(|b| b.is_ascii_hexdigit()) {
        anyhow::bail!("a subject is 64 hex characters");
    }
    parse_upstream(from)?;

    let mut follows = load_follows(store)?;
    match follows.iter_mut().find(|f| f.subject == subject) {
        Some(existing) => {
            let mut changed = false;
            if !existing.from.iter().any(|e| e == from) {
                existing.from.push(from.to_string());
                changed = true;
            }
            // Upgrading relay -> reader is a real change; downgrading is not
            // something a caller asks for by omitting an argument.
            if purpose.is_some() && existing.purpose.as_deref() != purpose {
                existing.purpose = purpose.map(str::to_string);
                changed = true;
            }
            // A fresh invite is the subject saying where they are NOW. Somebody
            // who moved off a public relay onto their own re-shares their code,
            // and the point of that is for it to take effect.
            if relay.is_some() && existing.relay.as_deref() != relay {
                existing.relay = relay.map(str::to_string);
                changed = true;
            }
            // **A BUNDLE IS ONLY EVER ADDED, NEVER CLEARED BY OMISSION.** A v2
            // invite for somebody already followed carries none, and reading
            // that as "they no longer have a keys vault" would drop the one
            // value a reader needs to join — from a record it may never get
            // back, because the invite that carried it has been scanned and
            // thrown away. Rotation is a real change and takes effect; absence
            // is not a statement.
            if keys.is_some() && existing.keys.as_deref() != keys {
                existing.keys = keys.map(str::to_string);
                changed = true;
            }
            if changed {
                save_follows(store, &follows)?;
            }
            Ok(changed)
        }
        None => {
            follows.push(Follow {
                subject,
                from: vec![from.to_string()],
                purpose: purpose.map(str::to_string),
                relay: relay.map(str::to_string),
                keys: keys.map(str::to_string),
            });
            save_follows(store, &follows)?;
            Ok(true)
        }
    }
}

/// Offer a subject we already follow a `diaswarm-keys` identity, and prove it
/// is ours.
///
/// **THE READER'S HALF OF [D27](../../docs/decisions.md).** This peer was
/// granted on the subject's old vault long ago. The new vault needs an identity
/// the subject has never seen, and only the reader can supply one — with a
/// proof only the two of them can produce, because a tag is readable by anyone
/// holding a replica of the grant log.
///
/// Tries the subject's upstreams in order and stops at the first that takes it.
/// A subject too old to understand the request replies empty, which reads as
/// "not taken" — and is exactly right: the reader goes on reading the vault it
/// already reads, and nothing anyone relies on stops.
pub async fn hand_over_to(
    store: &Path,
    subject_hex: &str,
    mine: &diaswarm_core::vault::Identity,
    keys_identity: &str,
    endpoint: Option<(&iroh::Endpoint, &[u8])>,
) -> Result<bool> {
    let follows = load_follows(store)?;
    let Some(follow) = follows.into_iter().find(|f| f.subject == subject_hex) else {
        anyhow::bail!("not following {subject_hex}");
    };
    // The purpose this peer reads under. A relay follows without one and has
    // nothing to hand over: it was never granted anything.
    let Some(purpose) = follow.purpose.as_deref() else { return Ok(false) };

    let Ok(bytes) = diaswarm_core::vault::unhex(subject_hex) else {
        anyhow::bail!("a subject is hex")
    };
    let subject_pub: [u8; 32] =
        bytes.try_into().map_err(|_| anyhow::anyhow!("a subject is 32 bytes"))?;
    let proof = diaswarm_core::vault::hex(&diaswarm_core::seal::handover_proof_for(
        &mine.encryption,
        &subject_pub,
        purpose,
        keys_identity,
    ));
    let tag = diaswarm_core::vault::hex(&diaswarm_core::seal::grant_tag(
        &mine.encryption,
        &subject_pub,
        purpose,
    ));

    for upstream in &follow.from {
        let Ok(addr) = parse_upstream(upstream) else { continue };
        let sent = match endpoint {
            Some((ep, alpn)) => {
                crate::wire::hand_over(ep, alpn, addr, &tag, keys_identity, &proof).await
            }
            None => {
                let ep = iroh::Endpoint::bind(iroh::endpoint::presets::Minimal).await?;
                let alpn = crate::swarm::default_wire_alpn();
                crate::wire::hand_over(&ep, &alpn[..], addr, &tag, keys_identity, &proof).await
            }
        };
        if matches!(sent, Ok(true)) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// What one refresh of one subject did.
#[derive(Debug, Clone)]
pub struct Refreshed {
    pub subject: String,
    /// The endpoint that answered, if any.
    pub via: Option<String>,
    pub segments: usize,
    pub wraps: usize,
    /// Why each upstream that was tried did not work.
    ///
    /// Kept rather than discarded because "the subject looks offline" and
    /// "every peer refused" are different problems with different fixes, and
    /// an earlier version of this reported both as silence.
    pub failures: Vec<(String, String)>,
}

impl Refreshed {
    pub fn reached(&self) -> bool {
        self.via.is_some()
    }
}

/// Refresh one subject, trying each upstream until one answers.
///
/// Stops at the first success. The bytes are identical wherever they come
/// from — sealed segments, wraps nobody in between can open, and a signed
/// grant log — so there is nothing to gain by asking a second peer, and no
/// reason to prefer the subject over anyone else holding them.
/// Refresh one subject using this peer's own endpoint.
///
/// Preferred over [`refresh_one`] wherever a peer has one: a throwaway endpoint
/// per fetch means the pool sees an identity it knows nothing about, and one
/// that stops existing the moment the sync ends.
pub async fn refresh_one_on(endpoint: &Endpoint, store: &Path, follow: &Follow) -> Refreshed {
    refresh_inner(store, follow, Some((endpoint, &crate::swarm::default_wire_alpn())), false).await
}

/// Refresh one subject over an endpoint that answers on a non-default ALPN.
///
/// A peer in the pool does: p2panda mixes the protocol id with its network id.
/// See [`crate::wire::fetch_on_alpn`].
pub async fn refresh_one_on_alpn(
    endpoint: &Endpoint,
    alpn: &[u8],
    store: &Path,
    follow: &Follow,
) -> Refreshed {
    refresh_inner(store, follow, Some((endpoint, alpn)), false).await
}

/// Refresh everything, using this peer's own endpoint.
pub async fn refresh_all_on(endpoint: &Endpoint, store: &Path) -> Result<Vec<Refreshed>> {
    let follows = load_follows(store)?;
    let mut out = Vec::new();
    for f in &follows {
        out.push(refresh_inner(store, f, Some((endpoint, crate::ALPN)), false).await);
    }
    Ok(out)
}

pub async fn refresh_one(store: &Path, follow: &Follow, local_only: bool) -> Refreshed {
    refresh_inner(store, follow, None, local_only).await
}

async fn refresh_inner(
    store: &Path,
    follow: &Follow,
    endpoint: Option<(&Endpoint, &[u8])>,
    local_only: bool,
) -> Refreshed {
    let into = store.join(&follow.subject);
    let mut failures = Vec::new();

    for from in &follow.from {
        let addr = match parse_upstream(from) {
            Ok(a) => a,
            Err(e) => {
                failures.push((from.clone(), format!("{e:#}")));
                continue;
            }
        };
        // NOTHING ABOUT WHO THIS PEER IS. A relay and a reader send byte-identical
        // requests and receive byte-identical replies; the only difference is
        // which of the wraps they can afterwards open, and that is decided
        // locally by which keys they hold. Making relay and reader
        // indistinguishable on the wire is what lets a peer hold a friend's
        // history without that being visible to anyone it talks to.
        let attempt = match endpoint {
            Some((ep, alpn)) => fetch_on_alpn(ep, addr, alpn, &follow.subject, &into).await,
            None => fetch_with(addr, &follow.subject, &into, local_only).await,
        };
        match attempt {
            Ok((segments, wraps)) => {
                return Refreshed {
                    subject: follow.subject.clone(),
                    via: Some(from.clone()),
                    segments,
                    wraps,
                    failures,
                }
            }
            Err(e) => failures.push((from.clone(), format!("{e:#}"))),
        }
    }

    Refreshed {
        subject: follow.subject.clone(),
        via: None,
        segments: 0,
        wraps: 0,
        failures,
    }
}

/// Refresh everything this peer follows, one pass.
pub async fn refresh_all(store: &Path, local_only: bool) -> Result<Vec<Refreshed>> {
    let follows = load_follows(store)?;
    let mut out = Vec::new();
    for f in &follows {
        out.push(refresh_one(store, f, local_only).await);
    }
    Ok(out)
}

/// The subject key this identity would be followed under, for printing.
pub fn own_subject(identity: &Identity) -> String {
    hex(&identity.enc_public())
}
