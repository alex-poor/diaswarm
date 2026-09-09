# Decisions

What is settled, why, and what evidence would reopen it. Reasoning lives in
[feasibility.md](feasibility.md); this is the index so a decision is not quietly
re-litigated six weeks from now.

**`D1`–`D9` here are decisions. `RQ1`–`RQ10` in feasibility.md §8 are the
requirements frameworks are judged against.** Both were `D` until a review found
that `D3` meant *"audit is given up"* in this document and *"data survives for
years"* in the other one — in two documents that cite each other on every page.

---

### D1 · Records are public ciphertext, not private and pull-only

**Settled.** A swarm exists to make data available to your people *whether or not
your phone is on*. Anything that answers reads from the author's device — a
Holochain private entry, a remote zome call — makes the subject a personal
server, which is the thing a distributed store exists to fix.

So records are published and replicated, therefore encrypted, therefore access is
**key custody**. "Named recipients" is a statement about keys, not about storage.

*Reopens if:* the flagship use case turns out to need read-auditing more than
availability. See D3 — that is the trade being made.

### D2 · Framework is p2panda

**Settled**, on the three layers a design needs: storage swarm, **key layer**,
grant records. p2panda is the only candidate supplying all three.
`p2panda-encryption` *data mode* — shared group key, rotated on removal, joiners
given prior secrets deliberately — is the requirement described in the library's
own terms.

Holochain is a good storage swarm with **no key layer for public entries** (cap
grants gate zome calls, not DHT reads). Hypercore's read key is irrevocable.
Meadowcap gates sync, not rest.

*Reopens if:* `p2panda-spaces` stalls past the point of usefulness. Fallback is
OpenMLS for the key layer — audited, an RFC, and its Delivery Service problem
mostly dissolves because the subject's phone is the only writer of its own data.

⚠️ **One claim this decision rested on is now measured false.** feasibility.md
§8.4 said the library could "withhold history from a researcher while granting it
to a clinician", because history-on-join is a per-join choice. It is not:
`add()` welcomes a joiner with the entire secret bundle, and a member added at
the end of the spike opened every epoch including six from before it existed.
**Time-scoped grants need one group per window**, which is more group management
than the design assumed. This does not reverse D2 — the key layer is still the
only one on offer — but it moves work into the integration.

✅ **Cadence checked 2026-09-08, and that condition is not met — the opposite.** v0.7.1
was published **21 August 2026, eighteen days ago**, with 0.5.0/0.6.0/0.7.0 in
January, May and July of the same year. **`p2panda-spaces` is published** at
0.7.1, and its tracking issue is closed. What remains open — documentation,
key-bundle auto-rotation, concurrent encryption messages, group promotion and
demotion — are open items on a released crate.

This decision was previously annotated with the reverse conclusion, from a date
in feasibility.md that was off by a year. **The recommended track is in better
shape than the documents said**, and OpenMLS stays a fallback rather than
becoming live. Re-check again before committing to the sealing work, since a
crate three weeks old is a moving target in a different way.

*Watch:* proxy re-encryption (Umbral/TACo lineage). Uniquely allows granting
access to already-published data without the subject's device participating,
which is what a commons enrolling at scale would want. Too immature to build on;
TACo is being forked and relaunched H2 2026 — **which is now**, so this is a
thing to go and look at rather than a thing to wait for.

### D2a · The key layer ships as the reference construction, not p2panda

**Settled 2026-09-09, amending D2.** D2 chose p2panda for the key layer. The
spike that tested it (`spike/p2panda-seal`) confirmed the property holds and that
revocation is *finer* than the epoch — and also that **`add()` cannot scope
history on join**: a member added at the end opened every epoch, including six
from before it existed. Time-scoping therefore needs one p2panda group per
window.

**D11 made that the flagship's problem, not a research edge case.** *"A clinician
gets the last 90 days"* is a window. Under per-recipient wrapping a window is
free — it is simply which keys you wrapped. Under p2panda it is a group to
create, join, rotate and revoke, per grant.

So the epoch-and-wrap construction is what ships: `crates/diaswarm-core::seal`,
ported from `tools/seal.py` and **byte-compatible with it**, verified by sealing
in one language and opening in the other in both directions.

**This is a deferral, not a rejection.** The key layer sits behind one seam —
`epoch_key`, `wrap`, `unwrap` — so p2panda can replace it without touching the
record path, the plugin, or the file format. What changes is that it is no longer
on the critical path, and the project no longer waits on someone else's 0.x.

**When to go back to p2panda — three questions, not one.** D2 chose p2panda for
*all three* layers. D2a defers only the middle one, and the other two are
untouched and arrive on different schedules.

| Layer | Status | The trigger that brings p2panda back |
|---|---|---|
| **Storage / transport** | **Untouched, and next.** Nothing replicates anything yet | **The moment two devices must sync.** The file-based demo is the last stage that avoids this. D2's reasoning here never depended on the key layer, and `p2panda-net` over iroh 1.0 is still the answer |
| **Keys** | Deferred (this decision) | Any of the four below |
| **Grants** | Signed Ed25519 records in `seal.rs`; `p2panda-auth` is the alternative | Comes with the transport, since a grant record has to replicate like anything else |

**The four things that would reopen the key layer**, in the order they are likely
to bite:

1. **Multi-device (§12.5).** A phone and a spare is a group with two writers, and
   keeping key material in step between them is what a CGKA is actually for.
   Per-recipient wrapping does this badly. **Not optional — loop phones get
   replaced**, so this is a when, not an if.
2. **Cohort scale.** Wraps are O(readers × epochs). At five readers, 164 KB/year
   — nothing. At a thousand, 92 KB *per epoch*, about 33 MB/year, and every
   enrolment rewraps history. A commons at scale is where this construction stops
   being obviously right.
3. **Revocation granularity.** p2panda's `remove()` rotates immediately; this
   rotates at the epoch boundary. If asking real people shows *"they keep the
   rest of the day"* is unacceptable (D4's open question), p2panda is already
   better on the axis the design cares most about.
4. **The review gate (10.6).** If a reviewer's answer is "use something audited",
   that is the answer, and it was D2's own argument.

**And a standing recheck:** `p2panda-spaces` is actively released — 0.7.1 was
three weeks old when measured. The gap that caused this deferral is a missing
subset argument on `add()`, which is a small thing to fix upstream and might
simply close. **Re-measure before starting transport work**, because that is the
moment the two decisions have to agree anyway.

⚠️ **Still not reviewed cryptography.** X25519 + HKDF-SHA256 +
ChaCha20-Poly1305, Ed25519 for grants, composed by hand. D2's fallback argument
— *"we extended an audited upstream" is an easier sentence than "we wrote our
own"* — now cuts against this choice, and stage 10.6's review gate matters more
because of it, not less.

### D3 · Audit is given up, deliberately

**Settled and uncomfortable.** Public ciphertext means anyone can pull it and
nobody can log a read. What survives is a signed, public, tamper-evident record
of **grants** — who was given what, over what range, and when it stopped.

That is less than "usage is audited" and more than the incumbent offers. It must
be described accurately, in the words in feasibility.md §11, because a person may
act on it.

*Exception:* the research commons (D5) fetches under grant and is an identifiable
organisation under an agreement, so the strong claim holds there — socially, not
cryptographically.

### D4 · Epoch keys, one UTC day each

**Settled, and now specified.** spec/records.md §5.1 freezes it:
`epoch = floor(t / 86400000)`.

**UTC, deliberately, and it costs something.** An epoch must have the same
identity on every device — a local-midnight boundary is ambiguous across travel
and DST, and two peers disagreeing about which epoch a record belongs to is a
correctness problem in a replicated store. The price is that away from UTC the
boundary falls inside the waking day (in NZ, near noon), which makes *"they keep
the rest of the epoch"* harder to say plainly to the person deciding whether that
is acceptable. A wording problem in one place against an ambiguity problem
everywhere.

*Reopens if:* asking real people about revocation granularity (below) shows the
"rest of the epoch" promise is unsayable at a noon boundary. The fix would be a
per-subject fixed offset, not local time.

A content key per epoch, wrapped to each live grantee and published beside the
data. Granting starts the wrapping; revoking stops it. This buys time-scoped
access for free (give a researcher one year's keys), and it bounds what a revoked
reader keeps to **one epoch**.

**Cost is now measured, not estimated.** The reference snapshot spans 49 epochs:
245 wraps for five readers, about **24 KB of key records beside 1.66 MB of
data** — 179 KB/year against the 180 KB/year §7.2 predicted. So cost is
irrelevant to the choice, as claimed. **Choose on revocation granularity.** Nobody has yet asked a person whether "they keep up to 24 more
hours" is acceptable, and that is a question for people, not for this repo.

### D5 · The commons is a gateway, not a bigger phone

**Settled.** Researchers will not run a p2panda node, so the endpoint is an
impedance match: an ordinary granted peer on the swarm side, ordinary research
formats (CSV, Parquet, the OPEN / OpenAPS Data Commons shapes) on the other.

**The framework choice stops at that box**, which is what makes a wrong framework
choice survivable.

It is also the way data comes **back** — cohort baselines, and what your data
supported. Build that in the first version: a commons that only takes is the
thing people have already refused.

*Must not become:* the default route, an identity broker, or a holder of history
it was never granted.

### D6 · Canonicalisation is part of the protocol

**Settled.** A dump of the AAPS database is not a person's history — it is the
history plus its own edit log. Version rows (`referenceId IS NOT NULL`) and
retracted rows (`isValid = 0`) are removed at the emit boundary, never
downstream, because a consumer that takes the tables at face value over-counts
insulin and carbs and biases every model fitted on them.

Loop telemetry (`deviceStatus`, `apsResults`) is excluded: two thirds of the
file, no clinical content, and the tables most likely to hold something nobody
meant to share. There is no flag to re-include it — the record vocabulary is
closed, and the flag that claimed to do this never did anything.

**Units are normalised at the same boundary, for the same reason.** AAPS stores
glucose values and temporary targets in mg/dL but profile blocks in whichever
unit the user set, so an un-normalised stream carries the same kind of quantity a
factor of 18 apart in two different records, separated only by a flag. That is
the failure spec/records.md §2 exists to prevent, and it was live in the emitter
until it was measured. A unit the emitter does not recognise is passed through
untouched *and* marked, because a visible gap beats a plausible wrong number.

### D7 · The AAPS plugin is read-only, structurally

**Settled and non-negotiable.** A `DataSyncSelector` drains a queue outward.
Nothing in that shape can write back into the loop, and nothing here may acquire
the ability. Anything that can influence dosing is inside the medical device's
blast radius and inherits its entire risk posture.

### D8 · A 0.x dependency is acceptable

**Settled**, reversing the assumption inherited from the whanau_voice work
(*"you can freeze a three-opcode protocol you wrote, and you cannot freeze
someone else's 0.x crate"*).

That rests on peers who never update. AAPS has none: `VersionCheckerPlugin`
applies expiry as a loop constraint — `maxIob.set(0.0, …)` — so an expired build
**stops dosing**. Every actively looping install is on a recent release.

*What survives:* followers are not loopers and have no expiry mechanism. That
compatibility burden lands at the gateway (D5), which has an administrator.

### D9 · Extend upstream, do not fork

**Settled.** Gaps get contributed: `p2panda-spaces` (rotation, expiry, credential
unification, concurrent auth messages) is already the project's own tracking
issue and already the blocker. NLnet/NGI funds this space and funded p2panda's
group-encryption work specifically, so the gap-filling is plausibly fundable in
its own right.

Engage upstream *before* writing the patch. Extensions age better than forks
against a moving codebase.

### D19 · The pool is the only way peers find each other

**Settled 2026-09-10, superseding D18.** There is one discovery mechanism:
p2panda's. The vault protocol carries no `Announce` and no `Holders`, no peer
writes another peer's address to disk, and `holders.json`, `peers.json` and the
public-address filter are gone. Wire version `diaswarm/5`.

**A follower falls back to the pool instead of to a list.** It joins the bucket
its followed subject falls into, hears holders announce themselves on that
topic, and dials one *by node id* — reaching it is p2panda's problem, and no
address is recorded or passed on. That preserves the property D18 was built for,
which is the only reason D18 could be deleted:
`a_follower_survives_the_subject_leaving_without_a_second_address` scans one
code, kills the subject, and still reads.

Every announcer is remembered, not the latest. The fallback matters precisely
when a peer has gone quiet, and the peer that has gone quiet is exactly the one
a single-entry table is most likely to be holding.

**THE COST D18 NAMED IS SMALLER BUT NOT GONE.** Nobody can ask a peer who else
holds a subject any more, and no IP address is written down anywhere. What
remains is that bucket topics are public: joining one and listening tells you
which subjects are announced there and by whom. That is a coarser social graph
than a per-subject holder list — it names holders, who are mostly strangers
holding ciphertext, rather than followers — but it is not nothing, and D18's
warning survives in that reduced form.

**Two things came out of the deletion that were bugs, not cleanup:**

  * **A pooled peer never answered on `diaswarm/5`.** p2panda hashes the
    protocol id with its network id before iroh sees it, so the follow path was
    dialling an ALPN nothing listens on — and the refusal, "peer doesn't support
    any known protocol", reads exactly like the subject's phone being switched
    off.

    **Caught one install before it mattered, and worth recording as a near
    miss.** The mismatch arrived with the commit that moved serving into the
    pool and deleted the old router; until then both ends used the plain string
    and following worked. The phones were still two commits behind that, so
    their followers kept working and nothing looked wrong — the code on `main`
    was broken and the hardware said it was fine. Measured with a probe that
    reproduced the shipped configuration exactly: plain ALPN refused, mixed ALPN
    reached.

    `swarm::wire_alpn` derives what actually goes on the wire and serve, fetch
    and refresh all use it, so a CLI peer and a pooled phone speak the same
    protocol. The derivation is p2panda's and private, so
    `an_address_dial_reaches_a_pooled_peer` fails loudly if it ever changes.
  * **diaswarm has its own network id.** p2panda's default is shared with every
    application using the library, and pool size decides bucket depth, which
    decides what every peer holds — so counting strangers running unrelated
    software is not cosmetic. It also keeps a test run out of the real pool: on
    this wifi, `cargo test` was joining the phones' pool and could have adopted
    a real person's ciphertext.

**A peer has one identity.** Serving and fetching share an endpoint, so the id a
peer is known by is the id it answers on. Kept from D18; a throwaway endpoint
per fetch is an identity the pool knows nothing about.

*Reopens if:* bucket-topic membership turns out to leak more than the holder
list did, or a follower needs to reach a subject the pool has never heard of and
discovery cannot resolve.

### ~~D18 · Peers tell each other who holds what, and that publishes the follower set~~

**Superseded 2026-09-10 by D19**, and deleted from the code. Kept here because
the leak it documents was real, was found on hardware, and the reasoning is what
a reader needs in order to judge whether D19's smaller version is acceptable.

**Was:** a peer records whoever fetches a subject from it, and will tell anyone
who asks. A follower folds those addresses into the list it tries. Wire version
`diaswarm/4`.

**Without it the swarm does not exist.** A follower knew exactly one address —
the one it scanned — so the moment that device slept there was nowhere else to
ask. The relay case was demonstrable but never happened: it worked because a
person typed a second address in. Everything else in this design is about
availability not depending on one phone, and it depended on one phone.

**THE COST IS THE FOLLOWER SET, and it is not small.** Anyone holding a
subject's public key can now ask any peer who else carries that subject, and
get back stable, dialable endpoint ids.

> **Amended 2026-09-10, from the first holder entry ever recorded on real
> hardware.** It was worse than this said. Peers announce addresses as well as
> ids so that two devices on one wifi can find each other with the uplink down,
> and the announcement included the phone's PUBLIC address — a home IP,
> geolocatable, handed to anyone who knows the subject key and asks. An
> endpoint id is a pseudonym; an IP address is a place.
>
> Only local addresses are advertised now — RFC1918, link-local, unique-local,
> loopback — which is the only case addresses were for. Public ones are
> resolvable through discovery anyway, so announcing them bought nothing and
> cost a home address. Filtered at both ends, so a peer cannot get a public
> address stored by announcing one. That is a social graph: not what was
said, but who is close enough to someone to be watching their glucose. It is
the leak feasibility.md §9 already named as the price of a swarm, arriving
through the one door that makes availability work.

Two things bound it, neither of which makes it go away:

  * **A peer can only ever announce itself.** The identity recorded comes from
    the authenticated QUIC connection, never from the message, so nobody can
    add a third party — which would otherwise make discovery a way to aim
    followers at an address of an attacker's choosing. Addresses ARE claimed,
    but reaching one still requires a handshake matching the announced id, so a
    lie costs one failed dial.
  * **Thirty-two entries, oldest out first.** A bound, not a policy: an
    unbounded list is somewhere to write as much as you like into someone
    else's storage.

**A peer has one identity.** Fetching used to bind a fresh endpoint per sync,
which was harmless until something depended on it — and then a subject
faithfully recorded the address of a peer that had already ceased to exist.
Serving and fetching now share an endpoint, so the id a peer is known by is the
id it answers on.

*Reopened, and closed the other way.* The follower set did matter more than
availability — because availability turned out not to need it. The pool already
knows who holds what, so the holder list was buying a leak for a capability
p2panda supplies. See D19.

---

### D17 · A follower polls; it is not pushed to

**Settled 2026-09-09.** A phone that follows someone refreshes on a timer —
every two minutes while awake — rather than being notified when new data
exists.

The subject cannot push, and that is structural rather than lazy: a subject
does not know who follows them. Following is unilateral and needs no
permission, the grant log names nobody (D13), and a fetch says nothing about
who is fetching (D15). There is no list of followers to notify, by design.

So the connection has to be opened by the follower. It could be held open — a
subscribe request that streams notifications — and that is the right answer for
a foregrounded app. It is not an answer for a phone in a pocket: Doze closes
sockets and batches wakeups, so background freshness is Android's decision
whatever the protocol does. Polling and subscribing converge to the same place
in the background, and polling is far simpler.

**Two minutes because a CGM produces a reading every five.** Polling faster
mostly discovers nothing has changed, which costs a manifest and a connection —
an unchanged subject transfers no segments and no wraps. Measured on two
phones: 105s and 123s between unattended refreshes.

**Only on a phone that follows somebody.** The same code runs on a phone driving
an insulin pump, and waking it every two minutes to ask a question it has no
reason to ask is a battery cost for nothing.

WorkManager's periodic floor is fifteen minutes, which is useless for glucose —
a reading would be seen a quarter of an hour late. A one-time job can carry any
delay and re-arm itself; the periodic job stays underneath as the thing that
restarts the chain after the process is killed.

*Reopens if:* two minutes proves too stale in use, which would mean adding
subscribe for the foreground case rather than polling harder.

---

### D16 · An invite is one string, and it works in both directions

**Settled 2026-09-09.** Sharing is bootstrapped by
`diaswarm:1:<subject>:<endpoint>:<purpose>:<check>`, shown as a QR code.

It replaced moving two 64-character hex strings between two devices by hand,
where a single transposed character produces a key that is structurally perfect
and belongs to nobody — and the resulting failure is silence: the fetch reaches
no one, or reaches someone with nothing to say for you. Four bytes of checksum
turn that into a sentence a person can act on.

**The subject field is exactly the key a grant is made against**, so one code
serves both directions: scan someone's invite to follow them, or to share with
them. Those are opposites and both ordinary, so scanning asks which was meant
rather than guessing — and never does both, which would hand out access nobody
chose to give.

**An invite is not a secret and not a grant.** Anyone holding one can fetch
ciphertext and open none of it; access still requires the subject to grant that
specific key, on their own device. Leaking one costs what publishing a public
key costs.

The endpoint in it is a first contact, not an address of record: any peer
holding the subject serves identical bytes (D15), and a follower collects more
endpoints as it goes.

---

### D15 · A fetch says nothing about who is fetching

**Settled 2026-09-09.** A peer asks for a subject's whole vault — every
segment, every wrap, whoever they are for — and every peer sends the identical
request. Wire version `diaswarm/3`.

The first design asked for the wraps under one tag, which was the obvious thing
and was wrong twice.

**It made relaying impossible.** A peer holding a friend's history does not know
anyone else's tags, so it replicated segments and no wraps: a copy that opens
for nobody, which is not a replica of anything. The swarm demonstration on
2026-09-09 only worked because the relaying laptop was fetching *as the reader*
and so happened to pick up that reader's wraps. Writing the same scenario as an
honest test — a relay granted nothing — failed immediately.

**And it leaked.** Naming your tag tells the peer you dial exactly which entry
in the public grant log you are. D13 took the reader out of the log; asking for
that entry by name over the wire put it back, against a peer who is by design
not trusted. Now a relay and a reader are indistinguishable on the wire, and the
only difference is which wraps each can afterwards open — decided locally, by
which key they hold.

**Costs almost nothing.** About 100 bytes per reader per segment: 15 KB for a
year of one reader. The manifest carries a wrap count and wraps are never
re-issued, so an unchanged subject transfers nothing at all.

Verified in `crates/diaswarm-net/tests/transport.rs`: a stranger now receives
byte-for-byte the same vault as the granted partner — every segment and every
wrap — and still opens nothing. That is the architecture's claim in its
sharpest available form.

*Reopens if:* the number of readers grows enough that mirroring all wraps is no
longer negligible, which would need per-reader ranges in the manifest rather
than a return to naming tags.

---

### D14 · Transport is iroh, and it serves to anyone who asks

**Settled 2026-09-09.** `crates/diaswarm-net` moves a vault between peers over
iroh 1.1 — dial-by-endpoint-id, hole punching, relay fallback — which is what
feasibility.md §8.8 recommended and the one dependency there that passes RQ8.

**It authenticates nobody, deliberately.** A vault is sealed segments, wraps
encrypted to one reader, and a grant log that names nobody (D13). There is
nothing to withhold, and a transport that decided who to serve would reintroduce
exactly what §9.2 removed: access by who a server serves rather than by who holds
a key. Measured, on the phone's own 74-epoch history: a granted partner fetches
74 segments and 74 wraps and opens 33,279 records; a stranger fetches the same 74
segments, gets 0 wraps, and opens nothing.

**A fetch takes every segment, not the openable ones.** Holding ciphertext you
cannot read is the property §7.1 rests on — and taking only what you can open
would announce what you were granted, which is the leak D13 closed in the log,
reintroduced through traffic.

**The grant log replicates**, per D13's requirement, and the fetched copy is
verified against the subject's key rather than trusted.

*What this is not:* it is **pull, not gossip**. A reader asks and a subject
answers, so it is the wrong shape for the flagship — a follower needs data while
the subject's phone is asleep, and a phone in a drawer answers nothing. Push,
store-and-forward, and what an always-on peer holds are the next problem. §9.5
already says the subject's side is nearly free, because AAPS has paid for the
foreground service.

*Reopens if:* `p2panda-net` turns out to give the gossip and sync layer more
cheaply than writing it. It is built on the same iroh, so that swap is a
transport-layer decision and not a key-layer one — the seam D2a preserved.

### D13 · The grant log names nobody

**Settled 2026-09-09**, addressing what feasibility.md §12.0 called the sharpest
unsolved problem in the design.

> **Amended 2026-09-09.** The log replicates so that truncating it is
> detectable — and for most of a day it was not detectable by anybody who
> mattered. `meta.json` published the subject's X25519 key, used for sealing and
> wrapping; the Ed25519 key the log is actually *signed* with appeared nowhere
> in the vault. So the only party who could check the signatures was the
> subject, holding their own secret — the one party a tamper-evident log exists
> to hold to account. Every replica that tried reported `CHAIN BROKEN`, because
> it was verifying against a key that had never signed anything.
>
> The vault now publishes the signing key, `verify_own_chain` needs nothing
> from anywhere else, and a vault written before this heals on its next seal or
> grant — the two operations that hold the identity, so nobody has to be told
> to run a migration. Not being able to check now reports as an error rather
> than a pass, for the same reason §12.3 gives about a follower's readings.
>
> Found by replicating a log through a relay and asking a reader to verify it.

The log used to carry `{"purpose":"clinician","reader":"<public key>"}`. Two
leaks, and the second is worse than it first looks:

- the **purpose** is the most revealing word in the record — *clinician*,
  *cohort* — and it was in cleartext;
- a reader's public key is **the same key in every subject's log**. One clinician
  granted by fifty people appeared identically fifty times, which identifies them
  and clusters their patients.

**The fix is a tag derived from the Diffie-Hellman secret the two parties already
share**, with the purpose folded into the derivation:
`tag = HKDF(ECDH(subject, reader), "diaswarm-grant-tag-v1" || purpose)`. So it is
unlinkable across subjects, the reader can still compute their own and find their
entries, nobody else can compute either, and the purpose is never published. The
**wrap filenames** use the same tag — they leaked exactly as much and were easy
to miss.

Who is who lives in a **private book beside the subject's identity**, never
inside the directory that gets copied.

**What still leaks, and it is not nothing:** how many grant events a subject has
made and roughly when. A log with one entry and a log with forty are
distinguishable, and a burst of withdrawals looks like a burst. Hiding that needs
cover traffic, which is a different design.

**The tamper-evidence has two halves, and only one is local.** The entries are
hash-chained, so altering one or removing one from the middle is detectable from
a single copy. **Truncating the tail is not** — what remains is a valid prefix
and the subject holds every key needed to re-sign a shorter log.

**Replication closes that, and it is the swarm rather than a new mechanism.**
Once peers hold the log, truncating the local copy is not deletion but
*equivocation*: this copy says one thing, theirs says another, and the
disagreement is the evidence. feasibility.md §7.4 lists *"publication is
permanent"* as a **cost** — you cannot delete your data. Applied to the grant log
it is the **benefit**: you cannot delete your grants either. The same property,
read from the other side, and it is why D3's substitute for a read log is worth
anything at all.

⇒ **Therefore a requirement, recorded here so transport does not skip it: the
grant log must replicate to peers, not only the sealed data.** A few hundred
bytes an entry, so the cost is nothing — and without it the tamper-evidence §11
promises rests on the subject's own copy being honest, which is exactly the thing
it is supposed to establish.

Two limits survive replication and should not be talked past. An entry created
and dropped **before any peer saw it** leaves no trace anywhere: the guarantee is
*what was seen is permanent*, never *the log is complete*. And a subject can show
**different logs to different peers** — catching that needs peers to compare with
each other, which Certificate Transparency calls gossip, and which this design
gets only if peers actually do it.

### D12 · Followers are Android. iOS is out of scope

**Settled by the project's owner, 2026-09-08.** iOS cannot hold a background
socket, so an iPhone cannot be a peer. Every route to serving it — push, or a
relay that queues for an absent peer — ends at Apple's push service.

**Declining is more consistent with the design than serving it badly.** The whole
claim is that nobody in the middle can read or withhold anything (§9.4); putting
APNs in the delivery path would contradict it for the flagship's most visible
surface. feasibility.md previously called this the biggest single risk and
proposed answering it before designing the follower app. It is instead accepted.

*What it costs, plainly:* in most countries a large share of the people a subject
might share with are on iPhones, and they are excluded entirely. **This cannot be
recommended as a general replacement for Nightscout while that holds**, and any
description of it must say so rather than implying broad follower support.

*Reopens if:* adoption beyond Android becomes a goal, or someone is willing to
run a queuing relay and be named as the party in the middle.

### D11 · The flagship is personal sharing, not research

**Settled by the project's owner, 2026-09-08.** The point is **privacy and
sovereignty**: a person shares their own history with friends, family and
clinicians, revocably, with nobody in the middle. Research donation stays a good
use case and is not the first one.

This resolves an inconsistency rather than creating one — feasibility.md §6
already called "a parent seeing their child's CGM at 3 a.m." the flagship, while
§10.0 recommended research/cohort first because that is where the incumbent is
worst and where the strong audit claim survives. Both of those remain true; both
were the wrong reason to pick the first surface.

**What it costs, taken deliberately:**

- **The strong audit claim does not apply here.** Followers hold replicas, so
  what survives is grants and egress, never reads (D3). The §6 trade —
  availability at 3 a.m. over a read log — is now being made in the flagship, on
  purpose, and §11's wording is the primary promise rather than a footnote.
- **iOS followers become the biggest single risk**, not a conceded gap
  (feasibility.md §9.5). A parent with an iPhone is the flagship, and cannot be a
  peer.
- **Windowing lands in the main path.** "A clinician gets the last 90 days" is a
  window, and D2's measurement says p2panda cannot scope history on join.
- **The incumbent to beat is Nightscout, not Open Humans** — and on the
  subject's own side this is *easier* on convenience, not harder: Nightscout
  means a ~$5/month host plus a mandatory database, and a forced migration
  whenever a platform drops a tier. This means a toggle in an app already
  installed. The convenience losses are on the **follower** side — iOS above all
  — and feasibility.md §11 has the table.

*Reopens if:* the sovereignty argument turns out not to move anyone, and a
research group turns out to be willing to fund or host. That is an adoption
finding, not a technical one, and it would change which surface is built next
rather than anything below it.

### D10 · The sealing reference is per-recipient wrapping; p2panda is what ships

**Settled.** §7.3 already said per-recipient wrapping is "the floor and it is
sufficient" at five readers. `tools/seal.py` is that floor, built
framework-neutrally for the same reason `canon.py` was: it demonstrates the
property on real data today instead of gambling the demonstration on an
unfamiliar 0.x API, and it prices the construction — 164 KB/year for five
readers, against the 180 KB §7.2 estimated.

**It is a reference, not a competitor.** The shipping key layer is
`p2panda-encryption` data mode (D2). What this buys is that the port has a
behavioural specification to match rather than a paragraph, and that anything
p2panda does differently becomes a visible question about p2panda.

**Measured, 2026-09-08** (`spike/p2panda-seal`): p2panda 0.7.1 gives the same
property, and its revocation is **finer** than the epoch — `remove()` rotates
immediately rather than at the day boundary. So the reference's promise is the
pessimistic one, which is the safe direction to be wrong in.

*Reopens if:* the reference stays more expressive in a way that matters. It
already is in one way — **per-recipient wrapping scopes history by which keys you
wrapped, and p2panda's `add()` cannot scope history at all** (see D2). If windowed
research grants become the flagship, that difference decides whether this
construction rides on top of p2panda's transport rather than being replaced by
its group.

⚠️ **Not reviewed cryptography.** Standard primitives composed by hand. Stage
10.6 exists to catch exactly this, and nothing here should be described as
reviewed until it has been.

---

## Open, and blocking nothing yet

One line per open question in [feasibility.md §12](feasibility.md), in its
numbering, plus one this index adds. **This list previously held five of the
seven and renumbered them**, which is how an open question stops being tracked.

| §12 | Question | |
|---|---|---|
| **0** | ~~**Do grant records need to be private too?**~~ **Addressed 2026-09-09** — see D13. Grants are filed under a tag derived from the shared secret, so the log names no reader and no purpose, and the same reader is a different tag to every subject. What still leaks is *how many* grant events there are and roughly when | Largely closed; the residue is volume, not identity |
| 1 | **Does MLS tolerate a Delivery Service that is offline half the day?** The commit chain has to survive Doze, a flat battery and a week in a drawer | Decides whether D2's fallback is real |
| 2 | **Replicate or fetch**, per grant type rather than per architecture. A design that quietly picks one has picked the use case too | The D3 trade, applied case by case |
| 3 | ~~**What does a follower see when the phone has been dark six hours?**~~ **Partly answered 2026-09-09** — every reading is shown with its age, an unreachable peer is reported per endpoint with the reason, and a subject held-but-unreadable is distinguished from one that has sent nothing. What is still open is the six-hour case itself: Doze decides how dark a follower goes, and nothing yet warns that silence has lasted too long | The display distinguishes them; nothing yet *alerts* |
| 4 | **Cohort re-identification.** A 5-minute CGM trace is close to a fingerprint | The question an ethics committee asks first |
| 5 | **Multi-device.** A phone and a spare is the problem `p2panda-spaces` exists to solve, and it is not optional — loop phones get replaced | Blocked on the same gap as D2 |
| 6 | **Delegation.** Diabetes has minors and has emergencies. The delegate for a child is permanent; the delegate in an emergency is unplanned | |
| — | **Who operates the commons**, and does being a named, revocable peer actually change what an ethics committee thinks? That is the claim this design makes to that audience and it has never been tested on one | Not in §12; the gate on D5 |
