//! Being in the pool: find peers, work out your share, hold it.
//!
//! WHAT CHANGED, AND WHY IT IS SMALLER THAN IT LOOKS. `wire.rs` moves a vault
//! between two peers who already know about each other. It has no way to meet
//! anybody, so the pool it serves is whoever you handed an invite to. This
//! module supplies the missing half — membership — using p2panda-net, which
//! D2a said to use the moment two devices had to sync and which was then
//! reimplemented badly three times.
//!
//! **The vault protocol is untouched.** p2panda's `Endpoint` takes a protocol
//! handler and dials by node id, so [`crate::wire::VaultServer`] registers on
//! p2panda's endpoint and answers exactly the requests it always did. What goes
//! away is everything that was about *finding* peers: the holder list, the
//! address announcements, the public-address filter, `parse_upstream`. All of
//! it is p2panda's job and it does it properly.
//!
//! HOW A PEER DECIDES WHAT TO HOLD:
//!
//!   1. Join the presence topic. Everyone with swarm on is there; it carries no
//!      data and exists only to be somewhere to count.
//!   2. Pool size decides how finely the subject space is cut ([`pool`]) and
//!      which buckets are yours.
//!   3. Join those bucket topics. Announce the subjects you hold on them, and
//!      hear about the ones you should.
//!   4. Fetch what you should hold and do not have.
//!
//! Nobody asks you to hold anything and you are not holding it for anyone in
//! particular. **Holding is not reading**: a peer carries ciphertext for people
//! it has never met and cannot open a byte of it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use futures_util::StreamExt;
use p2panda_core::{Hash, Topic};
use p2panda_net::iroh_mdns::MdnsDiscoveryMode;
use p2panda_net::{AddressBook, Discovery, Endpoint, Gossip, MdnsDiscovery};
use serde::{Deserialize, Serialize};

use crate::pool;
use crate::wire::VaultServer;
use crate::ALPN;

/// What a peer says on a bucket topic.
///
/// Only ever "these subjects exist and I have them". Not who reads them, not
/// what is in them, and nothing a listener has to trust — a peer that lies
/// about holding something is found out by the fetch failing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BucketMessage {
    /// Subjects this peer holds that fall in this bucket, and where to get them.
    ///
    /// The sender names itself, which it could lie about. It buys nothing: the
    /// id is what a dial authenticates against, so a false one reaches nobody,
    /// and a peer claiming to hold something it does not is found out by the
    /// fetch coming back empty. Naming yourself is a hint, not a credential.
    Holding { from: String, subjects: Vec<String> },
}

/// A running member of the pool.
pub struct Swarm {
    store: PathBuf,
    endpoint: Endpoint,
    book: AddressBook,
    gossip: Gossip,
    /// Kept alive: dropping these stops discovery.
    _discovery: Discovery,
    _mdns: MdnsDiscovery,
    _presence: p2panda_net::gossip::GossipHandle,
    /// Bucket topics this peer has joined, kept for the life of the peer.
    ///
    /// **NOT RE-SUBSCRIBED EACH PASS.** Gossip is ephemeral: a message reaches
    /// whoever is listening at the time and is gone. Subscribing, publishing
    /// and then listening for a moment each pass means two peers only ever hear
    /// each other if their windows happen to overlap — which, tested, they do
    /// not. The subscription has to outlive the pass.
    joined: Arc<Mutex<HashMap<u64, p2panda_net::gossip::GossipHandle>>>,
    /// Subjects heard about on bucket topics, and everyone who said they had
    /// them.
    ///
    /// **A LIST, NOT THE LATEST.** One holder per subject was enough to decide
    /// what to adopt, and useless for the case that matters: a follower falls
    /// back to the pool precisely when a peer has gone quiet, and the peer that
    /// has gone quiet is exactly the one a single-entry table is likely to be
    /// remembering. Keeping every announcer means the fallback has somewhere
    /// else to try.
    heard: Arc<Mutex<HashMap<String, Vec<String>>>>,
}

impl Swarm {
    /// Join the pool, and start answering for what we hold.
    ///
    /// The signing key is the phone's existing node key, so a peer keeps the
    /// identity it already had — everything that ranks peers ranks them by it,
    /// and a new key would look like a departure and an arrival.
    pub async fn join(store: impl Into<PathBuf>, signing_key: p2panda_core::SigningKey) -> Result<Self> {
        Self::join_network(store, signing_key, default_network()).await
    }

    /// Join a named pool. Used by tests, so a test run is alone in its own.
    pub async fn join_network(
        store: impl Into<PathBuf>,
        signing_key: p2panda_core::SigningKey,
        network: p2panda_net::NetworkId,
    ) -> Result<Self> {
        let store = store.into();
        std::fs::create_dir_all(&store)?;

        let book = AddressBook::builder().spawn().await.context("address book")?;
        let endpoint = Endpoint::builder(book.clone())
            .signing_key(signing_key)
            .network_id(network)
            .spawn()
            .await
            .context("endpoint")?;

        // ACTIVE. Spawned without a mode it does nothing at all, and the
        // symptom is two peers on the same wifi never seeing each other —
        // indistinguishable from the network being broken.
        let mdns = MdnsDiscovery::builder(book.clone(), endpoint.clone())
            .mode(MdnsDiscoveryMode::Active)
            .spawn()
            .await
            .context("mdns")?;
        let discovery =
            Discovery::builder(book.clone(), endpoint.clone()).spawn().await.context("discovery")?;
        let gossip =
            Gossip::builder(book.clone(), endpoint.clone()).spawn().await.context("gossip")?;

        // The vault protocol, unchanged, on p2panda's endpoint.
        endpoint
            .accept(ALPN, VaultServer::new(store.clone()))
            .await
            .context("registering the vault protocol")?;

        let presence = gossip.stream(topic(pool::presence_topic())).await.context("presence")?;

        Ok(Swarm {
            store,
            endpoint,
            book,
            gossip,
            _discovery: discovery,
            _mdns: mdns,
            _presence: presence,
            joined: Arc::new(Mutex::new(HashMap::new())),
            heard: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// The iroh endpoint underneath, so the follow path uses the same identity.
    ///
    /// One peer, one endpoint. A phone that fetched from somewhere else would
    /// be a second identity in the pool that nothing else knows about.
    pub async fn iroh_endpoint(&self) -> Result<iroh::Endpoint> {
        Ok(self.endpoint.endpoint().await?)
    }

    /// The pool this peer is in. Two pools with different ids never meet.
    pub fn network_id(&self) -> p2panda_net::NetworkId {
        self.endpoint.network_id()
    }

    pub async fn node_id(&self) -> Result<String> {
        Ok(self.endpoint.endpoint().await?.id().to_string())
    }

    /// Everyone currently in the pool, this peer included.
    ///
    /// Read from the presence topic rather than remembered, because pool size
    /// decides how much every peer carries and a stale count means everyone
    /// quietly holding the wrong amount.
    pub async fn pool_members(&self) -> Result<Vec<String>> {
        let me = self.node_id().await?;
        let mut ids: Vec<String> = self
            .book
            .node_infos_by_topics([topic(pool::presence_topic())])
            .await
            .unwrap_or_default()
            .iter()
            .map(|n| n.node_id.to_string())
            .collect();
        if !ids.contains(&me) {
            ids.push(me);
        }
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    /// One pass: work out our share, say what we hold, take on what we should.
    ///
    /// Everything is recomputed from the pool as it is now. That is what makes
    /// the repair automatic — a peer that has gone is simply not in the list,
    /// so the share widens and the gap closes without anyone noticing a loss.
    pub async fn tick(&self) -> Result<TickReport> {
        let me = self.node_id().await?;
        let members = self.pool_members().await?;
        let depth = pool::depth_for(members.len());
        let mine = pool::my_buckets(&me, &members, depth, pool::REPLICAS);

        let held = subjects_held(&self.store);
        let mut announced = 0usize;

        for bucket in &mine {
            let handle = self.ensure_joined(depth, *bucket).await?;

            // Say what we have here, every pass — a peer that joined since the
            // last one has not heard it, and gossip does not repeat itself.
            let ours: Vec<String> =
                held.iter().filter(|s| pool::bucket_of(s, depth) == *bucket).cloned().collect();
            if !ours.is_empty() {
                let msg = serde_json::to_vec(&BucketMessage::Holding {
                    from: me.clone(),
                    subjects: ours,
                })?;
                let _ = handle.publish(msg).await;
                announced += 1;
            }
        }

        let wanted: Vec<(String, Vec<String>)> = self
            .heard
            .lock()
            .unwrap()
            .iter()
            .filter(|(s, _)| !held.contains(*s))
            .filter(|(s, _)| mine.contains(&pool::bucket_of(s, depth)))
            .map(|(s, from)| (s.clone(), from.clone()))
            .collect();

        Ok(TickReport {
            pool: members.len(),
            depth,
            buckets: mine.len(),
            held: held.len(),
            announced,
            wanted,
        })
    }

    /// One pass, and take on what it finds.
    ///
    /// Bounded per pass because this runs on a phone: a peer that has just
    /// joined a large pool would otherwise try to pull its entire share in one
    /// go, over mobile data, in a worker with a deadline. It catches up over
    /// several passes instead.
    pub async fn tick_and_adopt(&self, max: usize) -> Result<(TickReport, usize)> {
        let report = self.tick().await?;
        let mut taken = 0;
        for (subject, holders) in report.wanted.iter().take(max) {
            // Any holder serves identical bytes (D15), so the first that
            // answers is the right one and a refusal is not a failure.
            for from in holders {
                if self.adopt(subject, from).await.is_ok() {
                    taken += 1;
                    break;
                }
            }
        }
        Ok((report, taken))
    }

    /// Join a bucket topic once, and keep listening for as long as we are up.
    ///
    /// **Joined once and stayed joined**, not re-subscribed per pass. Gossip is
    /// ephemeral: a message reaches whoever is listening at the time and is
    /// gone. Subscribing, publishing and then listening for a moment each pass
    /// means two peers only hear each other if their windows overlap — which,
    /// tested, they do not.
    async fn ensure_joined(
        &self,
        depth: u8,
        bucket: u64,
    ) -> Result<p2panda_net::gossip::GossipHandle> {
        if let Some(h) = self.joined.lock().unwrap().get(&bucket).cloned() {
            return Ok(h);
        }
        let h = self.gossip.stream(topic(pool::bucket_topic(depth, bucket))).await?;
        self.joined.lock().unwrap().insert(bucket, h.clone());

        let heard = Arc::clone(&self.heard);
        let mut rx = h.subscribe();
        tokio::spawn(async move {
            while let Some(Ok(bytes)) = rx.next().await {
                if let Ok(BucketMessage::Holding { from, subjects }) =
                    serde_json::from_slice::<BucketMessage>(&bytes)
                {
                    // A node id that does not parse cannot be dialled, so it is
                    // not a hint — it is junk that would sit in the table
                    // looking like an answer.
                    if from.parse::<p2panda_net::NodeId>().is_err() {
                        continue;
                    }
                    let mut set = heard.lock().unwrap();
                    for s in subjects {
                        if !is_subject(&s) || pool::bucket_of(&s, depth) != bucket {
                            continue;
                        }
                        let who = set.entry(s).or_default();
                        if !who.contains(&from) {
                            who.push(from.clone());
                            // Bounded: a subject announced by thousands of
                            // peers must not become a list of thousands on a
                            // phone. Any of them serves identical bytes.
                            who.truncate(MAX_HEARD);
                        }
                    }
                }
            }
        });
        Ok(h)
    }

    /// Refresh everyone we follow, falling back to the pool when they are away.
    ///
    /// **THIS IS WHAT MAKES A GRANT SURVIVE THE SUBJECT'S PHONE.** A follower
    /// scanned exactly one address — the subject's — so on its own it is as
    /// available as that one device. There used to be a holder list for this:
    /// peers recorded who had replicated them and handed the addresses on. It
    /// worked, and it was a second, weaker discovery mechanism sitting beside
    /// p2panda's, which is how a home IP address ended up in a file on a phone.
    ///
    /// The pool already says who holds what. Joining the bucket a followed
    /// subject falls into means hearing its holders announce themselves, and
    /// dialling one is a node id and p2panda's problem — no address is written
    /// down here, and none is passed to anybody.
    pub async fn refresh_follows(&self) -> Result<Vec<crate::peer::Refreshed>> {
        let me = self.node_id().await?;
        let members = self.pool_members().await?;
        let depth = pool::depth_for(members.len());
        let follows = crate::peer::load_follows(&self.store).unwrap_or_default();

        // Listen where these subjects are talked about. Not our share — we are
        // not holding for the pool here, we are asking after someone specific.
        for f in &follows {
            let _ = self.ensure_joined(depth, pool::bucket_of(&f.subject, depth)).await;
        }

        let endpoint = self.iroh_endpoint().await?;
        let alpn = wire_alpn(self.endpoint.network_id());
        let mut out = Vec::new();
        for follow in follows {
            let mut r =
                crate::peer::refresh_one_on_alpn(&endpoint, &alpn, &self.store, &follow).await;
            if !r.reached() {
                // The scanned address is quiet. Somebody in the pool announced
                // holding this, and any holder serves identical bytes (D15).
                let holders: Vec<String> = self
                    .heard
                    .lock()
                    .unwrap()
                    .get(&follow.subject)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|h| *h != me)
                    .collect();
                for from in holders {
                    match self.adopt(&follow.subject, &from).await {
                        Ok((segments, wraps)) => {
                            r.via = Some(from);
                            r.segments = segments;
                            r.wraps = wraps;
                            break;
                        }
                        Err(e) => r.failures.push((from, format!("{e}"))),
                    }
                }
            }
            out.push(r);
        }
        Ok(out)
    }

    /// Everyone the pool has said holds this subject.
    ///
    /// Hints, not credentials: a peer that claims to hold something it does not
    /// is found out by the fetch coming back empty.
    pub fn holders_heard(&self, subject: &str) -> Vec<String> {
        self.heard.lock().unwrap().get(subject).cloned().unwrap_or_default()
    }

    /// Fetch a subject we ought to be holding, from any peer that has it.
    pub async fn adopt(&self, subject: &str, from: &str) -> Result<(usize, usize)> {
        let node: p2panda_net::NodeId = from.parse().context("not a node id")?;
        let conn = self.endpoint.connect(node, ALPN).await.context("connect")?;
        let into = self.store.join(subject);
        crate::wire::fetch_over(&conn, subject, &into).await
    }

}

/// What one pass found.
#[derive(Debug, Clone)]
pub struct TickReport {
    pub pool: usize,
    pub depth: u8,
    pub buckets: usize,
    pub held: usize,
    pub announced: usize,
    /// Subjects in our buckets we do not have yet, and everyone who has one.
    pub wanted: Vec<(String, Vec<String>)>,
}

/// How many announcers to remember per subject.
const MAX_HEARD: usize = 8;

/// What [`crate::ALPN`] actually looks like on the wire inside a pool.
///
/// p2panda hashes the protocol id together with its network id before handing
/// it to iroh, so two pools with different network ids cannot accidentally
/// speak to each other. A dial that carries the plain string is refused with
/// "peer doesn't support any known protocol" — which is indistinguishable, from
/// the outside, from the peer having gone away, and cost an afternoon once
/// already in this crate with a mismatched ALPN.
///
/// Derived rather than asked for, because p2panda keeps the mixing private.
/// `an_address_dial_reaches_a_pooled_peer` in `tests/swarm.rs` is what keeps
/// this honest: if p2panda ever changes the derivation, that test fails rather
/// than followers quietly losing the ability to make first contact.
pub fn wire_alpn(network_id: p2panda_net::NetworkId) -> Vec<u8> {
    Hash::digest([&ALPN[..], &network_id[..]].concat()).as_bytes().to_vec()
}

/// The pool everyone joins by turning swarm on.
///
/// **NOT p2panda's default.** The default is shared with every other p2panda
/// application, and on a network id everything else rests: it decides who is
/// counted in the pool, which decides bucket depth, which decides what each
/// peer holds. Sharing it with strangers running unrelated software means a
/// pool size that has nothing to do with how many people are storing diabetes
/// data, and peers announcing into topics nobody in this application is
/// listening to.
///
/// It is also what keeps a test run off the wifi's real pool. Every test builds
/// its own id and is alone in it; without that, `cargo test` on the same
/// network as a phone that has swarm on joins that pool and starts adopting a
/// real person's ciphertext.
pub fn network_id(name: &str) -> p2panda_net::NetworkId {
    *Hash::digest(format!("diaswarm/pool/1/{name}").as_bytes()).as_bytes()
}

/// The one real pool.
pub fn default_network() -> p2panda_net::NetworkId {
    network_id("")
}

/// What the vault protocol looks like on the wire in the real pool.
///
/// Every dial and every `accept` outside p2panda uses this, so a peer started
/// by the CLI and a peer that joined the pool speak the same protocol.
pub fn default_wire_alpn() -> Vec<u8> {
    wire_alpn(default_network())
}

fn topic(bytes: [u8; 32]) -> Topic {
    Hash::from_bytes(bytes).into()
}

fn is_subject(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Subjects this peer has on disk, whether or not it can read them.
fn subjects_held(store: &Path) -> Vec<String> {
    let Ok(dir) = std::fs::read_dir(store) else { return Vec::new() };
    dir.flatten()
        .filter(|e| e.path().join("meta.json").exists())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|s| is_subject(s))
        .collect()
}
