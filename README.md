# diaswarm

Share the data your insulin pump loop is already writing — with your partner, your
parent, your clinician — **revocably, and with nobody in the middle**.

> Your phone keeps looping exactly as it does now. What is added is the ability to
> hand someone a key to your record, and to take it back.

**Status: working proof of concept, running on a live closed loop.** An AAPS add-on
seals its own history and serves it; other devices replicate it and serve it onward;
a granted reader opens it from any of them.

Two things have been done end to end on real hardware, with real data:

* **Two phones.** A factory-reset Android phone, set up from nothing, scanned a QR
  code and showed the subject's live glucose — 117 mg/dL, one minute old, matching
  the number on the subject's own screen — over the open internet, with no server,
  no account and nothing typed. Two scans: one to follow, one to grant.
* **A relay.** A stranger peer, granted nothing, replicated 146 segments and all 219
  wraps, could open none of it, and served the complete history to a granted reader
  while the originating phone was switched off. 33,800 records over 73 days.
* **A pool.** Two phones on a wifi network, with nothing configured between them,
  found each other, agreed independently on the size of the pool, and each worked
  out the same share of the subject space to carry. Turning the feature on is the
  whole instruction — there is no address to exchange and nobody to ask.

Not reviewed cryptography. See [Limits](#limits) before trusting it with anything.

> **Not affiliated with, endorsed by, or part of the AndroidAPS project.** This is
> an independent add-on that compiles against AAPS's plugin interfaces. It contains
> no AAPS source. AndroidAPS is the work of its own maintainers, and problems with
> this add-on are not theirs — report them [here](https://github.com/alex-poor/diaswarm/issues).
>
> **No APK is distributed here, and none ever will be.** Running a loop is running
> a medical device you assembled; you build it yourself, from source you have read.
> Releases in this repository version *this project*, not AndroidAPS.

---

## Why

If you loop, your data already leaves your phone — usually to Nightscout, which
means running a web server, keeping it patched, and trusting whoever hosts it.
Sharing is all-or-nothing and mostly permanent: a URL and a token, handed out and
rarely taken back.

The alternative is normally framed as "self-hosting versus a company's cloud". That
is the wrong axis. The question is **who decides who can read** — and in both cases
it is whoever runs the server, not you.

diaswarm removes the server. Data is published as ciphertext to whatever devices
care to hold it. Access is decided by **who holds a key**, and by nothing else.
Withdrawing access does not ask a server to stop serving; it stops wrapping the
next key.

## How it works

```
records ──▶ epochs ──▶ segments ──▶ sealed ──▶ served to anyone
                          │                          │
                          └─▶ wrapped per reader ────┘
                                     │
                              grant log (signs, names nobody)
```

1. **Records** are normalised into a canonical stream — one line per event, stable
   bytes, defined in [`spec/records.md`](spec/records.md).
2. **Epochs** are days, cut at *your* fixed UTC offset rather than UTC, so a day is
   a day where you live.
3. **Segments** are the unit of key custody. A day can hold several; withdrawing
   access cuts a new one immediately rather than waiting for midnight.
4. Each segment is **sealed** under its own key (ChaCha20-Poly1305), and that key is
   **wrapped** separately to each reader (X25519 + HKDF-SHA256).
5. Grants are **signed and hash-chained**, and filed under a tag derived from the
   shared secret — so the log proves what happened without naming anyone in it.
6. Peers **hold what they follow and serve what they hold**. A peer stores segments,
   every reader's wraps, and the grant log — and can read none of it.

Revocation is the absence of a file. Nobody is told; the departing reader is simply
not wrapped for the next segment. What they already downloaded stays readable, and
nothing here pretends otherwise.

## Sharing, from the phone

An invite is one string, checksummed, that also fits in a QR code:

```
diaswarm:1:<subject-key>:<endpoint>:<purpose>:<check>
```

Its subject field is exactly the key someone would grant, so **one code works in
both directions**:

| | Phone A | Phone B |
|---|---|---|
| 1 | *Your invite* → show the code | |
| 2 | | *Scan a code* → **Follow them** |
| 3 | | *Your invite* → show the code |
| 4 | *Scan a code* → **Share with them** | |
| 5 | | *People you follow* → reading, and how old |

Scanning never guesses which direction was meant — following someone and sharing
with them are opposites, and both are ordinary — so it names the key and asks.

An invite is **not a secret and not a grant**. Anyone holding it can download your
ciphertext and open none of it.

### How live is it

A follower polls every **two minutes** while awake, which is comfortably under the
five-minute cadence a CGM produces. Measured, not intended: 105s and 123s between
unattended refreshes.

**Android decides the rest.** Doze batches background work, so two minutes means two
minutes while the phone is awake and something longer while it is in a pocket. A
follower phone should be excused from battery optimisation, or it will be quiet for
much longer than that. This is why every reading is shown **with its age** rather
than as a bare number that implies it is current: a follower's dangerous failure is
not an error on screen, it is a value that looks fresh and is nine hours old.

Nothing needs re-granting as time passes. Every segment sealed after a grant is
wrapped for that reader as it is written, so a follower keeps receiving data
indefinitely until it is withdrawn.

## The trade

The whole design is one bargain, and it is worth reading before anything else.

| You get | You give up |
|---|---|
| Data available to your people when your phone is off | Any record of someone *reading* — reads are invisible |
| Revocation enforced by key, not by a server's goodwill | Deletion. Publication is permanent |
| A signed, tamper-evident record of every grant | A leaked key never expires |
| No server, no hosting bill, no operator to trust | Discovery: you still have to exchange a code |
| Peers find each other, so one phone sleeping is survivable | **The follower set is public.** Anyone with your key can ask who holds your data |

Language that must never be used about this — and the true version of each claim —
is in [docs/feasibility.md §11](docs/feasibility.md). It is not decoration. Someone
may make a 3 a.m. decision based on what this says.

## Safety

The AAPS add-on **ships disabled** and is structurally incapable of dosing. It is a
`DataSyncSelector`: it drains a queue outward. It holds no pump reference,
implements no constraint, and has no path into the loop. The residual risk is not
dosing — it is that a plugin which fails to construct takes the app with it, and an
app that will not start is a loop that has stopped. That is why it is off by
default, why the constructor does nothing, and why the native library is not loaded
until something is actually shared.

## Try it without a phone

```sh
cd crates/diaswarm-core
cargo run --bin diaswarm -- keygen /tmp/me.id
cargo run --bin diaswarm -- keygen /tmp/friend.id
cargo run --bin diaswarm -- init /tmp/vault /tmp/me.id 12        # your UTC offset
cargo run --bin diaswarm -- seal  /tmp/vault /tmp/me.id records.ndjson
cargo run --bin diaswarm -- grant /tmp/vault /tmp/me.id "$(cargo run -q --bin diaswarm -- pub /tmp/friend.id)"
cargo run --bin diaswarm -- read  /tmp/vault /tmp/friend.id      # what they can open
cargo run --bin diaswarm -- log   /tmp/vault                     # every grant, verified
```

No AAPS database? `tools/mkfixture.py` builds a synthetic one, and
`tools/canon.py db --stats` turns it into records and reports what it dropped and
why.

Two peers, one relaying:

```sh
cargo run --bin diaswarm-net -- serve /tmp/store /tmp/node.key   # peer 1
cargo run --bin diaswarm-net -- keep  /tmp/store2 '<invite>'     # peer 2 follows
cargo run --bin diaswarm-net -- peer  /tmp/store2 /tmp/n2.key /tmp/friend.id
```

## Repository

```
spec/records.md          The wire contract. Versioned in-band
docs/feasibility.md      The assessment: architecture, costs, what must not be claimed
docs/decisions.md        What is settled (D1–D15), and what would reopen each

crates/diaswarm-core     Records, sealing, the vault, grants. The reference implementation
crates/diaswarm-net      Peers: serving, fetching, following, over iroh (QUIC)
crates/diaswarm-android  The JNI surface the phone calls
plugin/                  The AAPS add-on: settings screen, QR scanner, sync worker,
                         and the follower that keeps other people's history current

tools/canon.py           AAPS SQLite → canonical records, with a dropped-and-why report
tools/seal.py            The sealing construction in Python, byte-identical to Rust
tools/mkfixture.py       A synthetic AAPS database, so everything runs with no real data
```

The Python and Rust implementations are checked against each other over real record
streams, byte for byte, because two implementations that agree are evidence and one
implementation is an assertion.

## Building the AAPS add-on

The plugin lives here; AAPS itself stays untouched, on its own branch, in a git
worktree. You need the AAPS source, the Android SDK and NDK, and:

```sh
CAMAPS=/path/to/your/aaps-checkout ./plugin/build-apk.sh --install
```

It refuses to install if the signing certificate no longer matches the device,
because on a looping phone a mismatch costs you a pump re-pairing.

## Limits

- **The cryptography has not been reviewed.** Standard primitives, composed by
  hand. Do not rely on this where being wrong would matter.
- **No forward secrecy.** A key that leaks opens everything it was ever wrapped for.
- **Reads are invisible.** Nobody can tell you who has read their copy, or when.
- **First contact is manual.** You exchange a code. There is no directory and no
  way to search for a person. Peers do find *each other* after that, which is how
  a follower survives your phone sleeping — see below.
- **The follower set is discoverable.** Anyone holding your public key can ask any
  peer who else carries your data and get back their endpoint ids — pseudonyms, but
  stable ones. Not what was said, but who is close enough to be watching. This is
  the price of availability without a server, and it is not yet optional
  ([D18](docs/decisions.md)). Addresses are advertised alongside those ids **only
  when they are local** (same-wifi, no uplink); public addresses are never shared,
  because a home IP is a place rather than a pseudonym.
- **Metadata leaks.** The number of grants and roughly when they happened are
  visible in the log, even though who they name is not.
- **Background sync is at Android's mercy.** Two minutes while awake; Doze stretches
  it, and a follower not excused from battery optimisation will be much slower. There
  is no push — a subscribe-and-notify protocol would fix the foreground case and Doze
  would still govern the rest.
- **Storage only grows.** A follower keeps everything it fetches, about 25 MB per
  year per person followed. There is no pruning and no way to hold only recent days.
- **No pause.** The only controls are withdrawing the grant or turning the plugin
  off. There is nothing between "sharing" and "not sharing".
- **iOS is out of scope.**

## Licence

AGPL-3.0, matching AndroidAPS, whose interfaces the add-on compiles against.
