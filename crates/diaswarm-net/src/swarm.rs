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
    /// The same sentence about the *keys* vault's subjects.
    ///
    /// **A SEPARATE VARIANT, BECAUSE THE TWO VAULTS HAVE DIFFERENT SUBJECTS.**
    /// A core subject is an X25519 encryption key and a keys subject is the
    /// Ed25519 key its logs are authored under, so the same person is two
    /// different hex strings. Announcing them in one list would send every
    /// listener off to `adopt()` — the `wire.rs` pull — for a subject that
    /// vault has never heard of, and get an empty answer that looks exactly
    /// like a peer that has gone away.
    ///
    /// Older peers drop this on the floor: `serde_json` fails to match the
    /// variant, and the receive loop already ignores what it cannot parse.
    HoldingKeys { from: String, subjects: Vec<String> },
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
    /// The same, for keys-vault subjects. See [`BucketMessage::HoldingKeys`].
    heard_keys: Arc<Mutex<HashMap<String, Vec<String>>>>,
    /// Keys subjects this peer holds, as last reported by whoever owns the
    /// replicator.
    ///
    /// **PUSHED IN, NOT READ OFF DISK, AND THAT IS THE WHOLE AWKWARDNESS.**
    /// `subjects_held` can list the core vault's subjects because they are
    /// directories. A keys subject is rows in a SQLite log store, and the
    /// thing that knows which ones this peer carries is the replicator's own
    /// association set — which lives a layer up. So the owner tells the swarm
    /// what it carries, and the swarm announces it.
    keys_held: Arc<Mutex<Vec<String>>>,
    /// Until when this peer accepts an invite pushed at it. See `Request::Offer`.
    offers: Arc<std::sync::atomic::AtomicI64>,
    /// Kept so a subject followed AFTER startup can be seeded too — a scan
    /// happens while this is running, and restarting the app to make a new
    /// follow reachable is not something to ask of anybody.
    relay: Option<iroh::RelayUrl>,
}

/// Where a peer is reachable when it is not on your wifi.
///
/// **ONE DEFINITION, IN THE CRATE THAT OWNS THE INVITE.** The relay travels in
/// the invite now (D23), so the default belongs beside the format that carries
/// it — a second copy here would be a second thing to change, and the two would
/// disagree the first time somebody edited only one.
pub use diaswarm_core::invite::DEFAULT_RELAY;

impl Swarm {
    /// Join the pool, and start answering for what we hold.
    ///
    /// The signing key is the phone's existing node key, so a peer keeps the
    /// identity it already had — everything that ranks peers ranks them by it,
    /// and a new key would look like a departure and an arrival.
    pub async fn join(store: impl Into<PathBuf>, signing_key: p2panda_core::SigningKey) -> Result<Self> {
        // THE RELAY, EXPLICITLY, and not by way of `join_network`. This is what
        // a phone calls, and going through the test-facing constructor once
        // silently took every phone off the relay while every test stayed
        // green — the sort of regression that looks exactly like the network
        // being quiet. `bin/relaycheck` is what caught it and is the guard.
        Self::join_via(store, signing_key, default_network(), DEFAULT_RELAY).await
    }

    /// Join a named pool, on the local network only.
    ///
    /// **NO RELAY, BECAUSE THIS IS WHAT THE TESTS USE.** A test that dials
    /// n0's relay is measuring somebody else's uptime — `serve_with` says the
    /// same thing about the same trap — and a suite of them contends for the
    /// same network setup and goes flaky. Observed: adding the relay made a
    /// passing swarm test fail under parallel load and pass alone.
    ///
    /// Anything that wants to be reachable from another network names its relay:
    /// [`Swarm::join`] for the default, [`Swarm::join_via`] for a specific one.
    pub async fn join_network(
        store: impl Into<PathBuf>,
        signing_key: p2panda_core::SigningKey,
        network: p2panda_net::NetworkId,
    ) -> Result<Self> {
        Self::join_via(store, signing_key, network, "").await
    }

    /// Join a named pool through a named relay. Tests pass their own of both,
    /// so a test run is alone in its pool.
    pub async fn join_via(
        store: impl Into<PathBuf>,
        signing_key: p2panda_core::SigningKey,
        network: p2panda_net::NetworkId,
        relay: &str,
    ) -> Result<Self> {
        let store = store.into();
        std::fs::create_dir_all(&store)?;

        // **GIVE THE ADDRESS BOOK A STORE, WHICH IS WHAT IT IS FOR.**
        // `AddressBook::builder()` with no store keeps everything in memory, so
        // `node_infos_v1` and `topics2node_infos_v1` stay empty for ever and a
        // peer restarts knowing nobody. Measured on a laptop: `pool 4` reported
        // while the persisted node table held **zero** rows.
        //
        // p2panda's own description of what is being thrown away: the address
        // book "is an important tool to watch for transport information
        // changes, keep track of stale nodes and identify network partitions
        // which can be automatically healed". A peer that syncs with one node
        // and never another is that last sentence exactly.
        //
        // **ITS OWN FILE, NOT THE KEYS STORE.** `p2panda-store` builds its pool
        // with `max_connections(1)` and no busy timeout, so two pools on one
        // SQLite file is a writer-contention bug rather than an untidiness —
        // the note on `Pooled` in `diaswarm-android` says so at length. A
        // separate file costs nothing and shares nothing.
        //
        // Done here rather than as a parameter because `Swarm` already owns a
        // directory and there are 47 call sites that should not have to care.
        let book_url = format!("sqlite://{}", store.join("addressbook.sqlite").display());
        let book = match p2panda_store::SqliteStoreBuilder::new()
            .database_url(&book_url)
            .create_database(true)
            .build()
            .await
        {
            Ok(db) => AddressBook::builder().store(db).spawn().await.context("address book")?,
            Err(e) => {
                // A peer with an unpersisted address book still works; it just
                // forgets everyone on restart. Worth saying, not worth refusing.
                eprintln!("diaswarm: address book not persisted ({e}) — peers are forgotten on restart");
                AddressBook::builder().spawn().await.context("address book")?
            }
        };

        // SEED WHOEVER WE ALREADY KNOW, BEFORE THE ENDPOINT STARTS. A follower
        // scanned exactly one thing: the subject's node id. That is enough to
        // reach them anywhere, but only if something says where to look —
        // `Discovery` is a random walk and needs somewhere to walk FROM, and
        // mDNS is the same room. Marking them `bootstrap()` is what p2panda's
        // own example does with the one node id a user pastes in.
        let relay_url: Option<iroh::RelayUrl> = match relay.parse() {
            Ok(u) => Some(u),
            Err(e) => {
                // Not fatal: a phone with an unusable relay is exactly the
                // LAN-only phone we had before, which still works at home.
                eprintln!("diaswarm: relay url {relay:?} is not usable ({e}) — local network only");
                None
            }
        };

        let mut builder = Endpoint::builder(book.clone()).signing_key(signing_key).network_id(network);
        if let Some(url) = relay_url.clone() {
            builder = builder.relay_url(url);
        }
        let endpoint = builder.spawn().await.context("endpoint")?;

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
        // **THE MEMBERSHIP KNOBS, WHICH WERE NEVER CONFIGURED.**
        // `Gossip::builder(..).config(..)` takes a `GossipConfig` whose
        // `membership` is iroh-gossip's HyParView config, and passing none gave
        // us the paper's defaults: `active_view_capacity: 5`,
        // `shuffle_interval: 60s` — the latter commented "Wild guess" upstream.
        //
        // **THIS MATTERS BECAUSE SYNC SESSIONS COME FROM MEMBERSHIP.**
        // `spawn_membership_task` initiates a session only on a HyParView
        // `Joined` or `NeighbourUp` event, so the set of peers you exchange
        // data with is exactly your active view. A peer that is not sampled
        // into it is a peer you never sync with, and nothing later corrects
        // that on its own.
        //
        // A pool of a handful of phones should fit inside a capacity of five.
        // It is raised anyway, because the cost is a few more connections in a
        // network this size and the failure it guards against is a carrier that
        // silently holds a stale copy. `DIASWARM_ACTIVE_VIEW` and
        // `DIASWARM_SHUFFLE_SECS` exist so this can be measured rather than
        // argued about.
        let mut gossip_config = p2panda_net::gossip::GossipConfig::default();
        if let Ok(n) = std::env::var("DIASWARM_ACTIVE_VIEW").unwrap_or_default().parse::<usize>() {
            gossip_config.membership.active_view_capacity = n;
        }
        if let Ok(secs) = std::env::var("DIASWARM_SHUFFLE_SECS").unwrap_or_default().parse::<u64>() {
            gossip_config.membership.shuffle_interval = std::time::Duration::from_secs(secs);
        }
        let gossip = Gossip::builder(book.clone(), endpoint.clone())
            .config(gossip_config)
            .spawn()
            .await
            .context("gossip")?;

        // The vault protocol on p2panda's endpoint. Built before it is handed
        // over so the offer window stays reachable from here.
        let vault_server = VaultServer::new(store.clone());
        let offers = vault_server.window();

        endpoint
            .accept(ALPN, vault_server.clone())
            .await
            .context("registering the vault protocol")?;

        let presence = gossip.stream(topic(pool::presence_topic())).await.context("presence")?;

        let swarm = Swarm {
            store,
            endpoint,
            book,
            gossip,
            _discovery: discovery,
            _mdns: mdns,
            _presence: presence,
            joined: Arc::new(Mutex::new(HashMap::new())),
            heard: Arc::new(Mutex::new(HashMap::new())),
            heard_keys: Arc::new(Mutex::new(HashMap::new())),
            keys_held: Arc::new(Mutex::new(Vec::new())),
            relay: relay_url,
            offers,
        };
        // Tell the address book where everybody we already follow lives, before
        // anything asks. One implementation of that rule, not two.
        swarm.seed_follows().await;
        Ok(swarm)
    }

    /// The iroh endpoint underneath, so the follow path uses the same identity.
    ///
    /// One peer, one endpoint. A phone that fetched from somewhere else would
    /// be a second identity in the pool that nothing else knows about.
    pub async fn iroh_endpoint(&self) -> Result<iroh::Endpoint> {
        Ok(self.endpoint.endpoint().await?)
    }

    /// The p2panda endpoint and gossip this peer runs on.
    ///
    /// Handed out so replication can be built on the *same* ones. A second
    /// endpoint would be a second identity in the pool that nothing else knows
    /// about — the mistake D19 records, arriving from a new direction.
    pub fn parts(&self) -> (Endpoint, Gossip) {
        (self.endpoint.clone(), self.gossip.clone())
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
    /// Whether this node is reachable through a relay right now, in a word.
    ///
    /// **BECAUSE AN UNCONNECTED RELAY IS AN INVISIBLE OUTAGE.** A relay cannot
    /// push to a node that is not there, so a phone whose relay connection has
    /// gone is unreachable from any other network — while looking perfectly
    /// healthy from its own side, still sealing and still publishing. That is
    /// exactly the state the loop phone was found in after a night: no TCP
    /// connection at all, twelve hours into a process that had connected fine
    /// at startup.
    ///
    /// It is reported rather than repaired here on purpose. Re-creating an
    /// endpoint is a heavy, disruptive act and the right trigger for it is not
    /// yet known — first this has to be observable over days, on a phone that
    /// sleeps.
    pub async fn relay_state(&self) -> String {
        let Ok(ep) = self.iroh_endpoint().await else { return "unknown".into() };
        use iroh::Watcher as _;
        let status = ep.home_relay_status().get();
        if status.is_empty() {
            return "none".into();
        }
        // `RelayStatus`'s state field is private, so this reads its Debug —
        // ugly, and the alternative is guessing. If iroh makes it public this
        // becomes one match.
        let connected =
            status.iter().filter(|s| format!("{s:?}").contains("Connected")).count();
        if connected > 0 { format!("connected({connected})") } else { "disconnected".into() }
    }

    /// Tell iroh the network underneath us may have changed.
    ///
    /// **THIS IS THE 71-MINUTE OUTAGE OF 2026-09-15, AND IT IS OURS.** The loop
    /// phone left the house at 15:05, moved from wifi to mobile data, and its
    /// relay connection went and stayed gone until 16:16, when wifi came back.
    /// AAPS never faltered: it sealed 84 epochs into that hole, `holds` climbed
    /// 1470 -> 1560, `missing 0, lost 0, failures 0`. The data was made and
    /// kept. It simply had nowhere to go.
    ///
    /// The reason is upstream, documented, and deliberate. `netwatch`'s Android
    /// route monitor is a stub whose comment reads "Very sad monitor. Android
    /// doesn't allow us to do this", and its wall-time poll is set to an hour on
    /// mobile to save battery — and fires only on a *clock* jump, never a
    /// network one. So on Android iroh cannot see a network change at all. Its
    /// own docs say what to do instead:
    ///
    /// > some systems like android do not expose this functionality to native
    /// > code. Android does however provide this functionality to Java code.
    ///
    /// So Java has to say so, and this is the way in. Not calling it means a
    /// phone that changes network keeps sockets bound to the interface it just
    /// left: not a carrier blocking us, not doze, not the foreground service —
    /// a notification we never sent.
    ///
    /// Cheap and idempotent by upstream's own account — "even when the network
    /// did not change [...] there is no harm in calling this function" — so the
    /// callers err towards calling it.
    pub async fn network_changed(&self) {
        match self.iroh_endpoint().await {
            Ok(ep) => ep.network_change().await,
            // Nothing to notify yet. The endpoint takes its first look at the
            // network when it starts, so a change before that is already seen.
            Err(e) => eprintln!("diaswarm: no endpoint to notify of a network change ({e})"),
        }
    }

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
        let held_keys: Vec<String> = self.keys_held.lock().unwrap().clone();
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

        // KEYS SUBJECTS ARE ANNOUNCED SOMEWHERE THIS PEER MAY NOT HOLD, and
        // that is the difference between being a replica and being the source.
        // A phone always carries its *own* keys log (`keysCarryAll`), but its
        // own subject hashes wherever it hashes — which, in a pool of any size,
        // is usually not one of this peer's buckets. Announcing only into
        // `mine` would mean the one peer that definitely has the data is the
        // one peer that never says so, and no stranger would ever hear the
        // subject exists.
        //
        // Invisible at two peers, because `REPLICAS` (3) exceeds the pool and
        // everybody holds everything. It would have started failing silently at
        // about five — which is to say, at the first pool that was not a test.
        for bucket in announce_buckets(&mine, &held_keys, depth) {
            let handle = self.ensure_joined(depth, bucket).await?;
            let ours_keys: Vec<String> =
                held_keys.iter().filter(|s| pool::bucket_of(s, depth) == bucket).cloned().collect();
            if !ours_keys.is_empty() {
                let msg = serde_json::to_vec(&BucketMessage::HoldingKeys {
                    from: me.clone(),
                    subjects: ours_keys,
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

        let wanted_keys = self.keys_wanted_in(&mine, depth, &held_keys);

        Ok(TickReport {
            pool: members.len(),
            depth,
            buckets: mine.len(),
            held: held.len(),
            announced,
            wanted,
            wanted_keys,
        })
    }

    /// Keys subjects heard in our buckets that this peer is not carrying yet.
    ///
    /// **SEPARATE FROM [`Swarm::tick`] BECAUSE TICKING ANNOUNCES.** The caller
    /// that owns the replicator needs to ask this question every pass, and both
    /// apps already tick once a pass of their own accord. Asking through `tick`
    /// would publish a second round of gossip each time for an answer that is
    /// sitting in a map.
    pub async fn keys_wanted(&self) -> Result<Vec<(String, Vec<String>)>> {
        let me = self.node_id().await?;
        let members = self.pool_members().await?;
        let depth = pool::depth_for(members.len());
        let mine = pool::my_buckets(&me, &members, depth, pool::REPLICAS);
        let held = self.keys_held.lock().unwrap().clone();
        Ok(self.keys_wanted_in(&mine, depth, &held))
    }

    /// One definition of "wanted", used by both callers.
    fn keys_wanted_in(
        &self,
        mine: &[u64],
        depth: u8,
        held: &[String],
    ) -> Vec<(String, Vec<String>)> {
        self.heard_keys
            .lock()
            .unwrap()
            .iter()
            .filter(|(s, _)| !held.contains(*s))
            .filter(|(s, _)| mine.contains(&pool::bucket_of(s, depth)))
            .map(|(s, from)| (s.clone(), from.clone()))
            .collect()
    }

    /// Tell the pool which keys-vault subjects this peer is carrying.
    ///
    /// **THIS IS THE HALF OF D15 THE KEYS VAULT DID NOT HAVE.** Pool adoption
    /// announced and carried core-vault subjects only, so "any holder serves
    /// identical bytes, and a subject whose phone is asleep is still readable"
    /// was true of the vault being replaced and false of the vault replacing
    /// it. A follower could only ever get data from the subject's own phone.
    ///
    /// **AND IT IS WHY ANNOUNCING IS NOT A NEW LEAK.** A peer saying "I hold
    /// keys-subject Y" is only safe because strangers now hold Y too. If the
    /// subject and its followers were the only holders, this announcement
    /// would publish the follower set — exactly the leak
    /// [D18](../../docs/decisions.md) was retired for. The crowd is what makes
    /// "P holds Y" ambiguous between following it and merely carrying it, so
    /// the announcing half and the adopting half have to ship together. Do not
    /// land one without the other.
    pub fn set_keys_held(&self, subjects: Vec<String>) {
        let mut held = self.keys_held.lock().unwrap();
        held.clear();
        held.extend(subjects.into_iter().filter(|s| is_subject(s)));
        held.sort();
        held.dedup();
    }

    /// Who was heard announcing a keys subject, newest announcement last.
    ///
    /// The fallback path for a follower whose subject has gone quiet: any of
    /// these serves the same operations.
    pub fn keys_holders_heard(&self, subject: &str) -> Vec<String> {
        self.heard_keys.lock().unwrap().get(subject).cloned().unwrap_or_default()
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
        let heard_keys = Arc::clone(&self.heard_keys);
        let mut rx = h.subscribe();
        tokio::spawn(async move {
            while let Some(Ok(bytes)) = rx.next().await {
                let (from, subjects, into) = match serde_json::from_slice::<BucketMessage>(&bytes) {
                    Ok(BucketMessage::Holding { from, subjects }) => (from, subjects, &heard),
                    Ok(BucketMessage::HoldingKeys { from, subjects }) => {
                        (from, subjects, &heard_keys)
                    }
                    Err(_) => continue,
                };
                // A node id that does not parse cannot be dialled, so it is
                // not a hint — it is junk that would sit in the table
                // looking like an answer.
                if from.parse::<p2panda_net::NodeId>().is_err() {
                    continue;
                }
                let mut set = into.lock().unwrap();
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
    /// Accept a pushed invite for the next `seconds`, because the user just put
    /// their code on screen and is expecting somebody to scan it.
    pub fn expect_offer(&self, seconds: i64) {
        let until = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
            + seconds.max(0) * 1000;
        self.offers.store(until, std::sync::atomic::Ordering::Relaxed);
    }

    /// Hand our own invite to somebody whose invite we just scanned.
    ///
    /// `their_endpoint` is the endpoint field out of the invite they showed —
    /// a node id, optionally with addresses — and `their_relay` is the relay it
    /// named, so a peer reachable only through somebody else's relay can still
    /// be dialled.
    pub async fn offer_to(
        &self,
        their_endpoint: &str,
        their_relay: &str,
        ours: &str,
    ) -> Result<bool> {
        let mut addr = crate::peer::parse_upstream(their_endpoint)?;
        if let Ok(url) = their_relay.parse::<iroh::RelayUrl>() {
            addr = addr.with_relay_url(url);
        }
        let endpoint = self.iroh_endpoint().await?;
        let alpn = wire_alpn(self.endpoint.network_id());
        crate::wire::offer_on(&endpoint, &alpn, addr, ours).await
    }

    /// Offer a subject we follow our `diaswarm-keys` identity (D27).
    ///
    /// Through the pool's own endpoint, for the reason `refresh_follows` gives:
    /// a follower that dials only the address it scanned is exactly as
    /// available as the subject's phone, and a subject asleep is a subject
    /// unreachable.
    pub async fn hand_over(
        &self,
        store: &std::path::Path,
        subject: &str,
        mine: &diaswarm_core::vault::Identity,
        keys_identity: &str,
    ) -> Result<bool> {
        let endpoint = self.iroh_endpoint().await?;
        let alpn = wire_alpn(self.endpoint.network_id());
        crate::peer::hand_over_to(store, subject, mine, keys_identity, Some((&endpoint, &alpn)))
            .await
    }

    /// Tell the address book where a followed subject can be reached.
    ///
    /// Idempotent and cheap, and run on every pass rather than only at startup:
    /// following somebody happens by scanning a code while the app is running,
    /// and a follow that only became dialable after a restart would look like
    /// the network being broken.
    async fn seed_follows(&self) {
        for f in crate::peer::load_follows(&self.store).unwrap_or_default() {
            // THEIR RELAY, NOT OURS. Two people you follow can be reachable
            // through different relays — one public, one their family's — and
            // dialling the second through the first finds nobody. An invite
            // from before relays existed carries none, and falls back to
            // whichever this phone uses itself.
            let url = match f.relay.as_deref() {
                Some("") => continue, // said explicitly: direct connections only
                Some(r) => match r.parse::<iroh::RelayUrl>() {
                    Ok(u) => u,
                    Err(_) => continue,
                },
                None => match &self.relay {
                    Some(u) => u.clone(),
                    None => continue,
                },
            };
            for from in &f.from {
                let Ok(addr) = crate::peer::parse_upstream(from) else { continue };
                let _ = self
                    .book
                    .insert_node_info(
                        p2panda_net::addrs::NodeInfo::from(addr.with_relay_url(url.clone()))
                            .bootstrap(),
                    )
                    .await;
            }
        }
    }

    pub async fn refresh_follows(&self) -> Result<Vec<crate::peer::Refreshed>> {
        self.seed_follows().await;
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
        // **ONE SUBJECT AT A TIME WAS ONE SUBJECT BLOCKING THE REST.** This was
        // a sequential loop, and a follow whose phone is flat does not fail
        // fast: the dial waits out QUIC's own timeouts, and every subject
        // *after* it in the file waits with it. The flagship is a parent with
        // more than one child, so that is a second child's readings stopping
        // for a reason that has nothing to do with them — and the screen says
        // only that the data is old.
        //
        // Concurrent, and each one bounded. They are independent: every follow
        // writes into its own subject directory, and the two limits are
        // different questions — `PER_FOLLOW` is how long one unreachable phone
        // may cost, and the concurrency is how many dials a phone runs at once.
        //
        // Found by `one_unreachable_subject_does_not_stop_the_others`, which is
        // the kind of test that only exists because two of these were found by
        // hand on real phones first.
        const PER_FOLLOW: std::time::Duration = std::time::Duration::from_secs(20);
        let attempts = follows.into_iter().map(|follow| {
            let endpoint = endpoint.clone();
            let alpn = alpn.clone();
            async move {
                match tokio::time::timeout(
                    PER_FOLLOW,
                    crate::peer::refresh_one_on_alpn(&endpoint, &alpn, &self.store, &follow),
                )
                .await
                {
                    Ok(r) => (follow, r),
                    // A timeout is "not reached", which is what an unreachable
                    // peer already looks like — the ordinary condition of a
                    // swarm, not an error.
                    Err(_) => (follow.clone(), crate::peer::Refreshed::unreachable(&follow)),
                }
            }
        });
        let refreshed: Vec<_> = futures_util::future::join_all(attempts).await;

        let mut out = Vec::new();
        for (follow, mut r) in refreshed {
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
    /// The same for the keys vault: subjects heard in our buckets that this
    /// peer is not yet carrying.
    ///
    /// **NOT ADOPTED HERE, UNLIKE `wanted`.** Taking one on means associating
    /// its logs with a topic on a `Replicator`, which this module does not
    /// hold — see [`Swarm::set_keys_held`]. The caller that owns the
    /// replicator carries these and then reports back what it carries.
    pub wanted_keys: Vec<(String, Vec<String>)>,
}

/// Where to announce keys subjects: this peer's share, plus wherever the
/// subjects it actually holds happen to live.
///
/// Pure, and separate from [`Swarm::tick`], so the rule can be asserted without
/// standing up a pool big enough for `mine` to exclude anything — which is the
/// only size at which getting it wrong is visible.
fn announce_buckets(mine: &[u64], held_keys: &[String], depth: u8) -> Vec<u64> {
    let mut out = mine.to_vec();
    for s in held_keys {
        let b = pool::bucket_of(s, depth);
        if !out.contains(&b) {
            out.push(b);
        }
    }
    out.sort();
    out.dedup();
    out
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

pub(crate) fn is_subject(s: &str) -> bool {
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

#[cfg(test)]
mod announce_tests {
    use super::*;

    /// A PEER ANNOUNCES ITS OWN SUBJECT EVEN WHEN THE POOL DID NOT GIVE IT
    /// THAT BUCKET.
    ///
    /// The bug this exists for was invisible in the integration test and would
    /// have been invisible on the two phones here: with `REPLICAS` (3) at or
    /// above the pool size, every peer holds every bucket and the distinction
    /// never arises. It starts mattering at about five peers — the first pool
    /// that is not a test — and the symptom would have been a subject that
    /// nobody in the swarm ever hears about, which reads exactly like the
    /// feature not being there.
    #[test]
    fn a_peer_announces_its_own_subject_outside_its_share() {
        let depth = 8u8;
        // A subject, and a share that deliberately excludes wherever it lands.
        let mine_missing_it: Vec<u64> = (0..(1u64 << depth))
            .filter(|b| *b != pool::bucket_of(SUBJECT, depth))
            .take(4)
            .collect();
        const SUBJECT: &str =
            "4a2f00000000000000000000000000000000000000000000000000000000beef";

        let held = vec![SUBJECT.to_string()];
        let buckets = announce_buckets(&mine_missing_it, &held, depth);

        assert!(
            buckets.contains(&pool::bucket_of(SUBJECT, depth)),
            "a peer holding {SUBJECT} would never announce it: its bucket is not \
             in this peer's share, so the one peer that certainly has the data \
             is the one peer that never says so"
        );
        for b in &mine_missing_it {
            assert!(buckets.contains(b), "the peer's own share was dropped");
        }
    }

    /// And it does not invent buckets for subjects it does not hold.
    #[test]
    fn announcing_adds_nothing_when_there_is_nothing_held() {
        let mine = vec![3u64, 9, 11];
        assert_eq!(announce_buckets(&mine, &[], 8), vec![3, 9, 11]);
    }
}
