# Security review: what a carrier's disk gives up

[decisions.md](decisions.md) `D1` settles that records are **public ciphertext**,
so confidentiality is **key custody**, not storage secrecy: a carrier holds a
share of the pool and is supposed to be able to open *none* of it. That is a
claim about what is on the disk. This document tests it the only honest way —
by taking a carrier's disk and trying to read the data off it — and records what
an attacker with that disk actually gets.

Same rules as the rest of `docs/`: a verdict, and the evidence that would change
it. The verdict is that the confidentiality claim **holds**, and that the cost
paid for it — the metadata leak of [feasibility.md §11](feasibility.md) — is
**real and measurable on this disk**, exactly as advertised.

---

## Threat model

The attacker has **full filesystem read access** to a running carrier: the
always-on peer of [`crates/diaswarm-peer`](../crates/diaswarm-peer), a stolen
laptop, a backup, a subpoenaed server, a second local user. They are **not** a
grantee of any subject the carrier holds. The question is whether that access
yields anyone's diabetes plaintext, key material to obtain it later, or the
ability to forge records. Metadata visible without a key is expected (`D1`
publishes ciphertext); the review's job is to say *which* metadata, precisely.

Out of scope: a compromised *publisher* (the phone holds its own keys — losing
it loses the data by definition), and network-level observation (that is the
transport's threat model, not the store's).

---

## Method

Acquire, then analyse offline — the same steps the attacker has:

1. Find the store. The `diaswarm-peer.service` user unit points the binary at
   `$XDG_DATA_HOME/diaswarm`, i.e. `~/.local/share/diaswarm`
   ([main.rs](../crates/diaswarm-peer/src/main.rs) `default_dir`).
2. Snapshot every file, **including the `-wal`**, so the analysis sees a
   consistent view of uncheckpointed writes.
3. `sqlite3` for structure and counts; a short Python pass for entropy and a
   plaintext scan; `cryptography` to derive the node's public key from
   `node.key` and search for it as a wrapped-key recipient.

Nothing here needs privilege, a network, or the running process. That is the
point: it is what anyone holding the bytes can do.

---

## What is on the disk

`~/.local/share/diaswarm/`:

| File | What it is |
|---|---|
| `node.key` | 32 bytes — the node's `p2panda_core::SigningKey` (ed25519). **Identity only**: it signs this peer's own gossip and names it in the pool. It is not a decryption key for anyone's records. |
| `keys.sqlite` | The record log (the name is historical). At review time: **1840 operations** across two logs — `LOG_ID 0` data, `LOG_ID 1` control. |
| `addressbook.sqlite` | The peer directory — `node_infos_v1` for the pool members this carrier has met. |

The record log carried exactly the three subjects this carrier is configured to
hold (`--carry` in the unit's `carry.conf`):

| Subject | Author key | Data-log ops | Control-log ops |
|---|---|--:|--:|
| Loop phone | `b6b573c6…` | 1756 | 70 |
| Phone B (AAPS) | `44f8b3b4…` | 13 | — |
| Ayni | `bb650467…` | — | 1 |

---

## Verdict: the confidentiality claim holds

Four attacks, hardest-hitting first. Three of four **fail** — and the three that
fail are the ones that would expose health data or let it be forged.

| Attack | Result | Evidence |
|---|---|---|
| **Read plaintext off disk** | **Fails** | The 1756 loop-phone **data-log** bodies measure **7.9997 bits/byte** — indistinguishable from random. Zero health terms across 789 KB. The 111 KB outlier record (a backfill/profile blob by size) is ciphertext too (7.9983). |
| **Recover keys from the store** | **Fails** | `key_secrets_v1`, `key_registry_v1`, `groups_v1`, `spaces_v1` are **all empty**. A carrier never joins a group, so there is nothing to unwrap with. |
| **Use this node's own key as a recipient** | **Fails** | The node's derived pubkey (`ffa6148b…`) appears in **zero** of the 849 KB of wrapped-key envelopes. It was never made a grantee, so no message key is addressed to it. |
| **Forge or alter a record** | **Fails** | Every operation is ed25519-signed by the authoring device and body-bound via `payload_hash`. Forgery needs the phone's signing key, which is not here. |
| Read the header metadata | **Succeeds — by design** | See residual findings below. This is `D1`'s published cost, not a defect. |

The control log (`LOG_ID 1`) reads lower at **6.84 bits/byte** — it is structured
CBOR (the p2panda-encryption group-management protocol: `Welcome`, `Control`,
`members`, `Add`, `ciphertext`, `Hpke`, `ReceivedKey`), not health data, and its
`ciphertext` fields are the HPKE-wrapped keys the carrier cannot open.

*Reopens if:* any of those four key tables is ever non-empty on a
carrier — that would mean the peer is decrypting, i.e. it has been made a
grantee, which a pure carrier must never be. Worth a periodic assert.

---

## Residual findings — metadata and availability, not content

Ranked by what an adversary gains.

### 1. Social-graph leak · known, accepted, present

The unencrypted headers name **whose** records this box holds: the three author
keys above. Worse, `topics_v1` shows a **shared pool topic (`495E5991…`) common
to all three subjects** — so a disk-level observer learns not just the three
identities but that they are *one linked group*. This is precisely the leak
[feasibility.md §11](feasibility.md) accepts as the price of `D1`.

*Fix:* none at the store layer; it is inherent to replicating a shared log.
Mitigated only by carrying for strangers (`--adopt`), which is what keeps
"P holds Y" ambiguous between following Y and merely carrying it. A carrier run
with `--adopt 0` for a family publishes that family's graph outright.

### 2. Payload-size traffic analysis · low, bounded

Record sizes and per-device volumes are in the clear: a typical ~89 B reading
against 111 KB batch blobs, and 1826 records from one phone. That distinguishes
record *types* and gross activity even though content is sealed.

**Bounded by a real design choice:** there is **no wall-clock timestamp in the
sealed operation.** `p2panda-core` 0.7.1's `Header` carries none, and the
diaswarm extension is a bare `LogId` (u32) — [wire.rs](../crates/diaswarm-keys/src/wire.rs).
So timing leaks only through file mtimes and receipt order, not the record
itself; an attacker with a cold copy cannot read a clock off the data.

*Reopens if:* an extension ever adds a timestamp for ordering. Ordering is
`seq_num` + `backlink`; keep it that way.

### 3. `node.key` at rest · low impact, hardened

The identity secret sits unencrypted (no OS keyring, no passphrase). Theft lets
someone **impersonate this carrier** in the pool — participate in gossip, serve
as this node — but grants **no** read access to any health data, because the key
is not a decryption key. As of this review the key is `chmod 600` and the store
dir `chmod 700` (was `0644`/`0755`); the `~/.local/share` `0700` ancestor was
already the real barrier against other local users, so this is defence in depth.

### 4. Availability / selective omission · trust-on-carrier by design

A carrier can silently **drop, withhold, or serve stale** records. It cannot
forge (finding above), but the store alone does not guarantee a reader sees the
latest record — freshness lives in the reader/publisher layer, not here. A
single carrier is a liveness dependency, not a confidentiality or integrity one.

---

## Net

- **Confidentiality of health data:** intact. No plaintext, no keys, not a recipient.
- **Integrity:** intact. Signatures + body-hash binding.
- **Metadata / social graph:** leaks as `D1` says it does — verified on this disk.
- **Availability:** trust the carrier, or run more than one.

---

## Re-running this

```sh
D=~/.local/share/diaswarm
# 1. snapshot (include the -wal)
cp "$D"/keys.sqlite* "$D"/addressbook.sqlite* "$D"/node.key /tmp/loot/

# 2. is any key table non-empty? (must be 0 on a carrier)
for t in key_secrets_v1 key_registry_v1 groups_v1 spaces_v1; do
  sqlite3 /tmp/loot/keys.sqlite "SELECT '$t', COUNT(*) FROM $t;"
done

# 3. is the data log actually random? (want ~8.0 bits/byte)
python3 - <<'PY'
import sqlite3, math, collections
b=b"".join(r[0] for r in sqlite3.connect("/tmp/loot/keys.sqlite")
          .execute("SELECT body FROM operations_v1 WHERE log_id=x'00'") if r[0])
c=collections.Counter(b); n=len(b)
print("data-log entropy:", -sum(v/n*math.log2(v/n) for v in c.values()), "bits/byte")
PY
```

Any key table returning non-zero, or the entropy dropping well below 8.0, is a
finding — investigate before dismissing.

---

*First assessment: 2026-09-15, against the live carrier on `diablo`
(1840 operations held). Method is repeatable; append a dated section above for
each re-run rather than overwriting this one.*
