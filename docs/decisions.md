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

### D4 · Epoch keys, one day each at a fixed offset — and segments beneath them

**Settled**, and frozen in `spec/records.md` §5.1.

> ⚠️ **Amended 2026-09-10, retiring two stale claims — both stale in the
> direction that made revocation sound worse than it is.** What superseded them
> landed on 2026-09-09 in `4a8c6e2`, *"spec v3: epochs cut at a fixed offset, and
> revocation that does not wait"*, and this entry was never updated. So
> `spec/records.md`, the README and this index disagreed for a day about the two
> things a person deciding whether to trust revocation would actually ask.
>
> **This is the second time this file has drifted from a document it cites on
> every page.** The header records the first: D3 meant *"audit is given up"* here
> and *"data survives for years"* in feasibility.md. Found this time while
> writing [rights.md](rights.md), because an external standard asked the question
> in a shape that made the contradiction visible — which is an argument for
> measuring this design against outside standards more often than never.

**1. The epoch is not UTC. It is a fixed per-subject offset.** This entry
previously argued for raw UTC and named its own escape hatch: *"the fix would be
a per-subject fixed offset, not local time."* **That condition fired and the fix
was taken.** `tools/canon.py` derives the offset from the mode of AAPS's own
`utcOffset` column — where someone lives, rather than where they happened to be
when a snapshot was taken.

**The original argument survives, and is why the fix took this shape rather than
local time.** An epoch must have the same identity on every device, and two peers
disagreeing about which epoch a record belongs to is a correctness problem in a
replicated store — so a *local midnight* boundary, which moves with DST and with
travel, was never available. A constant recorded once is not local time. What it
buys is that the worst case lands while the subject is asleep instead of near
noon, which is the wording problem the old entry accepted and no longer has.

**2. A revoked reader does not keep the rest of the epoch.** The sealing layer
**cuts a new segment on withdrawal**, so what a reader keeps is bounded by *when
they were revoked*. An epoch may hold several segments; it stays the unit grants
and consumers speak in, but it is no longer the unit of key custody. Epoch length
is therefore a question of cost and scoping rather than of safety.

**So "it bounds what a revoked reader keeps to one epoch" is retired.** D10 saw
this coming from the other side on 2026-09-08 — p2panda's `remove()` rotates
immediately rather than at the day boundary, so the reference construction's
promise was *"the pessimistic one, which is the safe direction to be wrong in"*.
The reference then stopped being pessimistic, and nobody said so here.

---

A content key per epoch, wrapped to each live grantee and published beside the
data. Granting starts the wrapping; revoking stops it. This buys time-scoped
access for free — give a researcher one year's keys — and per-recipient wrapping
scopes history by which keys you wrapped, which is the expressiveness D2a and D10
both turn on.

**Cost is measured, not estimated:** 245 wraps for five readers, about **24 KB of
key records beside 1.66 MB of data** — 179 KB/year against the 180 KB/year
feasibility.md §7.2 predicted. Cost is irrelevant to the choice, as claimed.

*One figure to check.* This entry has said **49 epochs** and `spec/records.md`
§5.1 says **47** for the same snapshot and the same 245 wraps. 245 = 49 × 5, so
the likely reconciliation is **47 epochs holding 49 segments** — two days with a
mid-day re-cut, which is exactly what correction 2 above creates. Neither
document says "segments", so this is unverified arithmetic, not a finding.
Whichever is right, the KB figures agree and nothing downstream moves.

**The open question changed shape, and it is no longer granularity.** Granularity
is now good: revocation does not wait for a boundary. What nobody has asked a
person is whether **"nothing new will be sent after you stop it"** is an
acceptable meaning of *withdraw* — given that everything already downloaded stays
readable forever. feasibility.md §11 requires that wording because every
alternative is untrue; it does not establish that the wording satisfies anybody.
Still a question for people and not for this repo — [rights.md](rights.md) §12
has the route to some.

*Reopens if:* asking real people shows a withdrawal that recalls nothing is not
recognisable to them as withdrawal at all. **There is no protocol fix in that
direction** — the honest responses are wording, or a narrower default grant.

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

### D26 · Drop to `p2panda-encryption`, and keep segments

**Proposed 2026-09-12, not yet built.** The spaces migration would trade a vault
that does what the flagship needs for one that cannot express it
([migration.md](migration.md)). This is the way out, and it is one layer down
rather than a different library.

**THE PROBLEM IS `p2panda-spaces`' MESSAGE LAYER, NOT THE CRYPTOGRAPHY UNDER
IT.** `SpacesArgs::Application` carries `space_dependencies` — the previous tips
of its space — so application messages form a chain: reading today requires
having processed everything since the grant, the per-operation cost rises with
history already held (~21 µs × depth, measured), and a reader can neither join a
window part-way nor close one.

`p2panda-encryption`'s data scheme has none of that, and its own documentation
describes the shape we want:

> *"When sending new data into the group we look up the latest secret in the
> bundle ... and use XChaCha20Poly1305 as an AEAD to encrypt the payload with a
> random nonce. Next to each ciphertext the used nonce and group secret id is
> mentioned so other members of the group can decrypt the data."*

A secret is fetched by id — `SecretBundleState::get(id)`, documented as "retrieve
a secret to decrypt data where we know which secret id has been used".
`encrypt_data` and `decrypt_data` are public free functions over a
`GroupSecret`. **There is no chain and no ratchet.** Random access, which is
exactly what a follower reading the last day needs.

**So: `EncryptionGroup` for membership and secrets, and diaswarm keeps its
segments.** A segment on disk becomes `(ciphertext, nonce, secret_id)` instead of
`(ciphertext, per-reader wraps)`. Reading epoch N needs segment N and the secret
it names, and nothing else — the property `readcost.rs` measures today and the
one the spaces vault loses.

**What this still deletes, which is the whole point of [D20](#).** Epoch key
derivation, per-recipient X25519 wrapping, and the key agreement behind it all
become p2panda's: `create`, `add`, `remove`, `update`, and a welcome that hands a
joiner the whole secret bundle. SECURITY.md's headline warning is about that
code, and it still goes. What survives is the segment layout, which is file
naming rather than cryptography.

**Why not the alternatives.**

* *An abstraction over `p2panda-spaces`.* The chain is in the message type. A
  layer above it cannot remove a dependency the layer below records.
* *Willow / Meadowcap.* Willow is a data model with sparse ranges and Meadowcap
  is capabilities; neither supplies group key agreement, so the layer this
  decision is about would still have to come from somewhere. Possibly relevant
  to replication and to D19's topic leak, not to this.
* *Staying on `diaswarm-core`.* Works today and is measured, but leaves 1,159
  lines of hand-composed cryptography that nobody has reviewed, which is the
  thing D20 exists to remove.

⚠️ **Nothing here is built.** The APIs are read and quoted, not exercised. What
would settle it is a spike that creates a group, adds a reader, seals segments
under `encrypt_data`, removes the reader, and shows the next segment is
unreadable to them — plus `readcost`-style numbers showing the recent read stays
flat.

*Reopens if:* the welcome message turns out to carry the secret bundle in a way
that is itself linear in history, which would move the cost rather than remove
it.

### D25 · Sharing writes to the peer it dials, and that moved the wire to `diaswarm/6`

**Settled 2026-09-11.** `Request::Offer` carries an invite *to* a peer, so the
subject's single scan finishes the exchange in both directions (D16). Wire
version `diaswarm/6`.

**IT IS THE ONLY REQUEST THAT WRITES.** Every other one is read-only, which is
what makes serving to anyone safe (§9.2) and what D14 rests on. This one adds a
subject to what the receiver replicates, so an unsolicited offer is a stranger
making your phone carry their ciphertext. It is accepted only while the
receiver's owner has their invite on screen — a deadline, not a flag, because a
window that has to be closed is one somebody forgets to close. Accepting is not
reading and grants nothing.

**THE VERSION DID NOT MOVE WITH IT, AND THAT IS WHY THIS ENTRY EXISTS.** The
variant landed in `42e6567` and `a975f2a` on 2026-09-11 with `ALPN` left at
`diaswarm/5` — against the rule written directly above the constant, which D19
had already paid for once. The bump is this entry.

**The rule needed applying rather than reciting, because this break is quieter
than the one the rule was written for.** That one broke fetching. This one does
not: an older peer fails to decode the variant, replies empty, and `offer_on`
reads empty as *"they did not take it"* — indistinguishable from a closed
window. So the one-scan flow fails against an old follower, permanently, wearing
the face of an ordinary refusal, while the trend line keeps updating and hides
it. A loud failure at the handshake is worth more than a working graph beside a
pairing flow that silently cannot work.

**Bumped now because it is free now.** Every released follower postdates the
variant — `Offer` at 10:36, the first release build at 13:33 — so nothing
deployed is cut off. It stops being free as soon as somebody is running a
published Ayni, and per D8 a follower has no expiry mechanism to drag it
forward: the loop app stops dosing when it goes stale, and a viewer never does.

**One line, because nothing hardcodes the string.** `wire_alpn` derives what
goes on the wire by mixing `ALPN` with the network id, and every dial and
`accept` outside p2panda goes through it or `default_wire_alpn`. Verified:
`an_address_dial_reaches_a_pooled_peer` plus the transport, peer, swarm and
replicate suites, 17 tests, all passing on `diaswarm/6`.

*Reopens if:* a published follower exists that a future bump would strand, at
which point the question stops being "bump or not" and becomes whether the wire
needs a negotiated version rather than a fatal one.

### D24 · The follower is published, and built without jitpack

**Settled 2026-09-11.** The follower is released as a signed APK, built by CI
from two public commits, and the goal is F-Droid.

**THE README'S "NO APK, EVER" RULE IS SPLIT, NOT BROKEN.** It was written about
the app that drives an insulin pump and still holds for it: you assemble a
medical device yourself, from source you have read. The follower is a viewer —
separate `applicationId`, no pump drivers compiled in, structurally unable to
dose (D22) — and handing somebody a file they can install is reasonable in a way
that handing them a loop is not. The workflow names `Aapsclient` in every gradle
invocation and then checks the output directory, failing the run if a `full`
artifact appears: a comment saying "do not build the pump app" is not a control.

**AGPL IS WHY THE RELEASE NOTES NAME BOTH COMMITS.** The APK contains
AndroidAPS, so distributing it obliges publishing the corresponding source. The
AAPS fork and its `diaswarm-addon` branch are public for that reason, not as a
convenience.

**Signed with a key that is not the debug keystore.** The `loop` build type is
debug-signed on purpose — that is what keeps `install -r` an update and the pump
key alive — and its private half is public knowledge. A distributed app needs a
key nobody else holds, so the release is signed from GitHub secrets by a
separate 4096-bit key. Without those secrets CI still produces an artifact,
unsigned: installable by nobody, reviewable by anybody, which is the right
outcome for a fork or a pull request.

**JITPACK IS GONE, and that was the F-Droid blocker.** F-Droid will not build
against it, because it compiles arbitrary source at an arbitrary commit and
nothing from it is reproducible. Exactly one artifact came from it: QRGen,
declared `api` by `plugins:main` and imported by no file in the tree, left over
from the removed SmsCommunicator. The invite QR is now encoded with
`com.google.zxing:core`, already present for the scanner. `pingplacepicker` and
`kulid` looked like blockers and were not — declared in the version catalog,
referenced by nothing, never resolved.

**What still stands between this and F-Droid**, so nobody reads the above as
"done": F-Droid builds one repository and this is two, which needs `srclibs` in
the metadata; a tagged release with a monotonic versionCode; and their build
server has to complete an AAPS build. Only the dependency half is settled.

### D23 · Peers meet through a relay, not only on the same wifi

**Settled 2026-09-11.** `Swarm::join_via` configures an iroh relay on the
endpoint and seeds every followed subject into the address book as a bootstrap
node carrying that relay.

**WHAT WAS ACTUALLY SHIPPED BEFORE THIS.** No relay was configured at all —
`Endpoint::builder` starts with an empty relay map and nothing ever added to it.
Discovery was mDNS (same LAN) plus p2panda's random walk, which needs a peer it
can already reach to walk from. So every demonstration this project had given
was three phones on one wifi, and a follower that left the building could not
connect to anything. That is not a swarm; it is a LAN party, and the README
implied otherwise.

**THE DESIGN DOCUMENT HAD THIS RIGHT AND THE CODE DID NOT.** feasibility §9.1
says it plainly, and said it before any of this was built: *"Holochain has
bootstrap nodes, libp2p has rendezvous points. Two phones on mobile data cannot
find each other, or reach each other through carrier NAT, without something in
the middle that is on"*, and it settles on the claim *"no seeders, no accounts,
no operator, and nobody holding your data — not 'no servers'"*. The relay was
never a discovery; it was a requirement written down, not implemented, and then
papered over by a README that claimed a two-phone test had run "over the open
internet" when it had run on one wifi. The lesson is not about relays. It is
that a capability nobody tested got described as if it worked.

The fix is not clever, it is p2panda's own `chat.rs` example: `.relay_url(...)`
on the endpoint, and `NodeInfo::from(addr.with_relay_url(url)).bootstrap()` for
the one node id the user already has. A follower scans exactly one thing — the
subject's node id — and that is enough to reach them anywhere, but only once
something says where to look.

**A RELAY IS NOT A SERVER THE DATA LIVES ON.** It is STUN plus a fallback path:
it helps two phones behind NAT find a direct route, and carries packets only
until they do. It holds nothing, and what passes through it is ciphertext it
cannot open — epoch keys never leave the phones. What it *can* see is that two
node ids exchanged bytes and from which addresses. That is a metadata observer,
and it belongs beside the social-graph leak this design already accepts
(feasibility §9) rather than being waved through.

**IT IS MEANT TO BE REPLACED.** `n0` runs the default relays publicly, which is
what makes the app work when installed rather than after somebody stands up
infrastructure. Anyone who would rather not give a third party that metadata
runs `iroh-relay` and points the phones at their own; nothing else changes.
Asia-Pacific by default because that is where these phones are.

**DEMONSTRATED ON HARDWARE, 2026-09-11.** The subject was the live loop phone
on a mobile carrier with wifi switched off — address `10.238.221.41`, behind
Vodafone NZ's CGNAT — and the follower was a second phone on home wifi at
`192.168.88.213`. Two networks, no shared LAN, no mDNS possible. The follower
kept mirroring every two minutes with `newest 0m old`: 09:56:25, 09:58:27,
10:00:29, and the trend line ran unbroken through the 09:55 cut. Its screen read
9.4 mmol/L, 63 seconds old.

The relay connection is directly observable rather than inferred: on the rooted
loop phone, `ss -tnp` showed the AAPS process holding an ESTABLISHED socket to
`5.223.65.62:443` — and it re-established from the carrier address within
seconds of the wifi going down.

`bin/relaycheck` covers the same ground on a desktop, reporting `Connected` in
about four seconds.

**THE RELAY TRAVELS IN THE INVITE**, as of the same day. `diaswarm:2:` adds a
relay field, and `peer::Follow` stores it per subject. A relay compiled into the
binary is one that every phone must be rebuilt to change, which would make "run
your own" a release rather than a new invite. It is per subject and not a global
setting because two people you follow may be reachable through different relays
— one public, one their family's — and dialling the second through the first
finds nobody.

`diaswarm:1:` invites are still read, and mean the default relay. Ones already
scanned onto phones do not stop existing because the format grew a field. An
empty relay field means direct connections only: the local network and nowhere
else.

**Still open:** which relay to actually use. n0's public one is the default for
now, chosen so the app works when installed; the decision to move to a
self-hosted relay is now a change of invite, not of code. Note that
`iroh-relay` defaults to `AccessConfig::Everyone` — a self-hosted relay carries
anybody's traffic until it is given an allowlist of node ids or a shared token.

### D22 · A follower is a separate app, and the build flavour is the fence

**Settled 2026-09-10.** The thing this project is trying to beat shows somebody
else's trend line in an app. So a follower puts the person they follow onto the
AAPS glucose graph, by inserting their readings into AAPS's own `glucoseValues`
table — [`SwarmFollowerBg`](../plugin/src/main/kotlin/app/aaps/plugins/sync/swarm/SwarmFollowerBg.kt).
That is a write into the database of a medical device, and D7 says this add-on
does not do that. The reconciliation is that **the app which drives the pump
never runs it**.

**WHY THE RISK IS NOT "A WRONG LINE".** `CompatDBHelper` fires `EventNewBG` on
every glucose row that lands, `EventNewBG` extends `EventLoop`, and
`InvokeLoopWorker` runs the loop on exactly that event. A followed person's
glucose in a looping phone's database is not a display bug. It is dosing on
somebody else's blood.

**THE FENCE IS `Config.AAPSCLIENT`, WHICH IS THE BUILD FLAVOUR** — not a
preference, not a runtime check on the active pump, not "is the loop currently
enabled". Those are all states something else can change: a preference import, a
tap in the Config Builder, a bug in this add-on. A flavour cannot be changed by
any of them.

`aapsclient` also happens to be exactly the right shape for a follower:

* its `applicationId` is `info.nightscout.aapsclient`, so it **installs
  alongside** the loop app instead of over it. The follower is a second app with
  its own database — on the same phone, if you like — and nothing it writes is
  visible to the app holding the pump key.
* `Config.PUMPDRIVERS` is `full || pumpcontrol`, so no pump driver is compiled
  into it. There is no pump to command.
* `LoopPlugin` is `alwaysEnabled(config.APS)`, and `config.APS` is false there.

Two further barriers, because a single fence around this is not enough. Every
mirrored row is tagged in `ids.nightscoutId`, and the drain refuses to publish a
tagged row — otherwise a follower re-publishes the person they follow as their
own glucose, and whoever follows the follower sees the wrong person's blood
under their name. And nothing is mirrored until somebody says whose line it is,
unless exactly one person is followed and there is nothing to say.

**WHAT THIS COSTS.** A follower needs a second APK and a second identity, so the
person they follow has to grant that identity rather than the one in their loop
app. That is a real cost at a demo and it buys a property no amount of care in
this file could: the readings cannot reach the pump app, because they are not in
its database.

**Still unguarded:** an AAPS database exported from a follower and imported into
a looping phone carries the rows across. That is a foreign-database import into
a medical device, already catastrophic for reasons that have nothing to do with
this; the tag at least makes the rows findable afterwards.

### D21 · Replication is p2panda log sync over the pool's own topics

**Settled 2026-09-10.** `crates/diaswarm-net/src/replicate.rs` carries subjects
with `p2panda-net`'s log sync instead of [`wire.rs`](../crates/diaswarm-net/src/wire.rs)'s
`Have`/`Manifest`/`Grants`/`Segment`/`Wraps`.

**NOTHING IN THE POOL CHANGES**, which is what makes this small. A bucket topic
already carries a gossip announcement of *which* subjects live in it; log sync
carries *what they contain*, over the same topic. Gossip answers "who is in
this bucket", log sync answers "give me their operations". `pool.rs` — the one
piece p2panda supplies no equivalent for — is untouched.

A subject is an author and a log: `diaswarm-spaces` writes everything into log
0 of the subject's own key, so `associate(topic, subject_key, 0)` is the whole
of "carry this person's data". A peer holding a bucket associates every subject
it hears about there and the data arrives without anyone being asked.

Measured in `tests/replicate.rs`: a peer that was told nothing but a topic ends
up holding a stranger's operations, and cannot read a byte of them.

**RECEIVING IS NOT HOLDING**, and the gap between them is the trap here. Log
sync hands the application the operations it fetched and stops; storing them is
the application's decision. The first version counted `OperationReceived` events
and looked healthy — sync started, six operations and 2,602 bytes crossed, sync
finished, live mode began — with an empty store behind it. **A peer that
carries nothing while reporting healthy sync is the worst shape a bug in this
project can take**, so the sync event log is kept and exposed rather than
reduced to a count.

**What this buys beyond deleting a protocol:** live mode. After catch-up, new
operations are pushed over gossip rather than polled for, which is the answer to
[D17](#) and §12.3 — the two-minute follower poll, and a reading that is stale
without saying so.

**What it cannot do:** start a sync on demand. `SyncHandle::initiate_session` is
`#[cfg(test)]` upstream, with a TODO wondering whether to make it public. A peer
subscribes and waits for discovery. For the pool that is correct — nobody is
dialled, everyone is found — but **a follower that has just scanned an invite
gets its data when discovery gets round to it**, which is a worse first
impression than the current fetch-on-demand and has no workaround inside the
library.

**AND IT HOLDS A REAL HISTORY, SEALED BY AAPS ITSELF.** Shadow mode
(`SwarmBooleanKey.ShadowSpacesVault`) seals every record into both vaults and
logs whether they agree; a re-drain button resets the plugin's high-water marks
so the whole database is read again. On the loop phone, mid-loop:

```
15:55:00  high-water marks reset — re-reading everything
15:57:37  shadow sealed 35,897 of 35,897 — 1 windows, 0 readers
          74 epochs 20630..20706, 4 grants
```

**Two and a half minutes for 74 days**, every record into both vaults, zero
errors and no native crash. 3.2 MB on disk against the old vault's 8.3 MB — the
new one holds ciphertext in operation bodies rather than segments plus per-reader
wraps, though the two are not like for like while the old vault carries four
grants and the shadow carries none.

Shadow mode also survived an app upgrade mid-flight: passes either side of the
swarm29→30 install agree, under different pids. That is the first evidence the
identity and state survive a restart *inside AAPS* rather than in a self-test
binary, which is the failure that would invalidate every grant ever made.

**A discrepancy worth chasing, and not yet chased:** the device emitted 35,897
records where `tools/canon.py` produced 31,341 from a snapshot of the same
database taken three hours earlier. The gap is roughly the 3,694 CGM duplicates
canon.py drops per five-minute bucket, so the likely answer is that the two
debounce differently — but "likely" is not measured, and D3's whole point is
that two implementations agreeing is evidence while one is an assertion.

**IT RUNS ON THE PHONE.** `crates/diaswarm-spaces/src/bin/selftest.rs` is a
binary rather than a JNI call on purpose: pushed to `/data/local/tmp` and run
over adb, it touches AAPS not at all — no install, no plugin, nothing near a
pump — and answers the question "it cross-compiles" does not.

On the loop phone, a Pixel 7 running a closed loop at the time:

```
opens two vaults              44 ms
seals five days               31 ms   (1,440 records, 8 operations)
grants a reader               21 ms
reader opens every record    181 ms   (2,016 of 2,016, 0 panicked)
revocation stops the next day  ok
identity survives a restart    ok
on disk                      2.4 MB
```

Bundled SQLite in app storage, a tokio runtime and p2panda's state machinery
all work there. Phone B, idle, was three to four times slower — worth
remembering before reading anything into a single timing.

**Measured 2026-09-10, and it does not.** Five cold runs: a direct dial reaches
data in 32 ms, log sync in 3 seconds, 5 of 5 arriving. A hundredfold as a ratio
and a spinner as an experience. Keeping a hand-written fetch path alive to save
three seconds would be the opposite of the point.

*Reopens if:* first contact between peers on different networks — not measured,
and dependent on n0 discovery rather than mDNS — turns out to be far slower than
the local case.

### D20 · The vault moves onto p2panda-spaces; a window is a space

**Settled 2026-09-10.** `crates/diaswarm-spaces` replaces the sealing
construction with `p2panda-spaces`, which composes `p2panda-auth`
(capabilities), `p2panda-encryption` (the key layer) and `p2panda-store`
(SQLite persistence). It exists **alongside** `diaswarm-core`, not instead of
it, until the properties have been compared on real history.

**What goes, if it lands:** 1,159 lines of hand-composed cryptography — a
content key per segment, that key wrapped separately to every reader, the
signed hash-chained grant log, the unlinkable grant tags, and the on-disk vault
layout. Also the `Have`/`Manifest`/`Grants`/`Segment`/`Wraps` protocol, since
messages become `p2panda_core::Operation`s and replication becomes
`p2panda-sync`'s log sync. SECURITY.md's headline warning — standard primitives
composed by hand, reviewed by nobody — is about the code this deletes.

**What stays ours, with a reason rather than by default:** bucket assignment
(p2panda is topic-based and supplies no shard assignment, so `pool.rs` stays),
the AAPS record canonicalisation, the JNI surface and the Kotlin plugin.

**A WINDOW IS A SPACE, AND EVERY READER IS IN EXACTLY ONE.** A reader can
belong to only one of a subject's spaces — the second one they join hands them
no welcome and then panics them (`spike/p2panda-spaces` §5b). So membership is
never carried forward; instead **every day is published into every live
window**:

| Grant | How |
|---|---|
| with history | add the reader to window 0, which holds everything and keeps receiving |
| from now on | open a new window and add only them; it cannot contain what predates it |
| revoke | `remove` from their window, which rotates immediately |

**The cost is duplication: one copy of each day per live window.** A partner, a
parent and a clinician on different terms is three copies of the subject's year
rather than one — roughly 78 MB each. That is the price of the per-grant history
choice, and it must reach the README's storage numbers before this ships,
because it changes what a peer carrying a share of the pool is agreeing to.

**Three things the library only exposes under `test_utils`,** which is not a
feature a medical application should enable, so each is worked around on public
API and written down here because none of it is guessable:

  * `Space::*_persisted` and `Manager::set_groups_state`/`set_space_state` are
    test-only. The public API returns state the public API cannot store, and
    `AuthGroupState` is a private alias. The route is one layer down, on
    `p2panda-store`'s public `GroupsStore`/`SpacesStore` traits — which needs
    the key the global auth state is filed under, a private constant.
    `state_survives_a_reopen` round-trips through the manager's own public read
    path so that a drift in that constant fails loudly.
  * `SecretKey::from_bytes`/`as_bytes` are test-only, so the encryption identity
    can be neither exported nor restored as bytes. `Credentials` derives
    `Serialize`, so the vault owns a 0600 `credentials.json`. A device that came
    back from a reboot as a new member would invalidate every grant made to it.
  * `repair_spaces_persisted` is test-only, and repair is **required**: all of a
    subject's spaces share one global auth state, so after any auth-level change
    every other space is stale and the next membership change on a stale one
    panics. `spaces_repair_required` then `repair_spaces` before every auth
    operation — on readers too, since processing a space's messages gives a peer
    state for that space whether or not it is a member.

⚠️ **`p2panda-auth` panics rather than erroring** when an operation arrives
without its dependencies, and inconsistently — the same situation sometimes
returns a clean error. Log sync delivers in dependency order, which is the
answer, but `Vault::ingest` processes every operation inside `catch_unwind` and
**returns the panic count**, because on a looping phone a panic in a background
worker takes the app down, and a reader that silently received four days out of
five is the failure this project keeps being bitten by.

**MEASURED ON REAL HISTORY.** `crates/diaswarm-spaces/tests/differential.rs`
put 31,341 records — 74 days of this subject's actual loop history, 2.71 MB —
through both implementations and got the same records out of both, exactly.
That is the result that would justify deleting `vault.rs`.

**THE PAYLOAD RIDES IN THE OPERATION BODY, NOT THE HEADER.** `p2panda-spaces`
puts an application message's ciphertext inline in `SpacesArgs::Application`,
which lives in the operation header — and `p2panda-core` decodes headers with
`.length_limit(512)`. Following that as given was measured and was untenable:
one `profile` record in this history is 566 bytes, already over the ceiling, so
no record-boundary chunking could publish it at all; 2.71 MB became **10,569
operations**, 3× inflation on the wire, and a **quadratic** read — 5 ms per
operation at 537, 155 ms at 10,569, twenty-seven minutes for 74 days on a
laptop. A follower catching up on a year would not have finished.

An operation has a *body* for exactly this, and `Builder::body()` folds its hash
and size into the signed header. `SwarmForge` moves the ciphertext there and
`operation::Operation` restores it before the spaces layer reads the args —
which is what the `Forge` trait is for; its own documentation calls it the
"interface for wrapping forge args in custom message types". **No cryptography,
access control, key management or state handling changes: the bytes are
identical and still produced and consumed entirely by the library.** Only where
they sit in the envelope is ours to choose.

| | ciphertext in the header | ciphertext in the body |
|---|---|---|
| operations for 74 days | 10,569 | **79** |
| on the wire | 8.06 MB (3.0×) | **2.74 MB (1.01×)** |
| wall clock | 27 minutes | **5.1 seconds** |

The quadratic cost is still there — per-operation state handling grows with
history, in `process` and in writing state back roughly equally — but at 79
operations for 74 days it stops mattering. **It is a reason never to put bulk
data through a spaces message**, which is now a rule this integration follows
rather than a limit it hits.

*Not verified:* whether replication imposes its own body-size limit. Nothing
syncs these operations yet. `p2panda-blobs` is the crate meant for bulk content
and is unusable — published at 0.5.2 against everything else's 0.7.1, and an
empty stub in git pending a refactor — so if a body limit appears, this is where
the question reopens.

*Reopens if:* the duplication cost turns out to matter more than the per-grant
choice — in which case every reader goes in window 0 and "from now on" is
dropped — or if upstream lets a reader belong to more than one space, which
would remove the fan-out entirely.

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

**Settled 2026-09-09; the format grew a relay field on 2026-09-11 (D23).**
Sharing is bootstrapped by
`diaswarm:2:<subject>:<endpoint>:<purpose>:<relay>:<check>`, shown as a QR code
or pasted as text. `v1` invites are still read and mean the default relay.

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
| — | **Is "nothing new will be sent" recognisable as *withdrawal*?** D4's question, reshaped: granularity is solved, semantics are not. Nobody has asked a person whether a withdrawal that recalls nothing counts as one | Not in §12; rights.md §12 has a route |
| — | **Who operates the commons**, and does being a named, revocable peer actually change what an ethics committee thinks? That is the claim this design makes to that audience and it has never been tested on one | Not in §12; the gate on D5 |
