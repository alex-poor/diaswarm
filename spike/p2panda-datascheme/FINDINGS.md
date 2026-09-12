# Keys from p2panda, data in our own segments

**Run:** `cargo run --manifest-path spike/p2panda-datascheme/Cargo.toml`

[D26](../../docs/decisions.md) argued from reading the API that the scaling
problem is `p2panda-spaces`' message layer rather than the cryptography beneath
it, and that `p2panda-encryption`'s data scheme would let diaswarm keep its
independently-sealed segments. This exercises that instead of asserting it.

---

## 1. Arbitrary payloads, outside any p2panda message

180 days sealed under the group secret with `encrypt_data`, 2,078 KB of
ciphertext, largest segment **11,824 bytes**. A `SpacesArgs::Application`
message is capped at 64 KB *and* chains to its space's previous tips. These do
neither: a segment carries its ciphertext, its nonce and the id of the secret
that opens it, and refers to nothing else.

## 2. Reading the recent end is flat

| days held | newest day | whole history |
|---|---|---|
| 7 | 1.6875 ms | 11.86 ms |
| 30 | 1.6886 ms | 45.31 ms |
| 90 | 1.3173 ms | 107.62 ms |
| 180 | **1.1876 ms** | 214.09 ms |

**Flat across a 26× range of history**, and a full catch-up is linear. This is
the property `diaswarm-core` has, `p2panda-spaces` loses, and the parent case
needs — a follower reading the last day pays the same whether the subject has
been sealing for a week or six months.

For contrast, the same question of the spaces vault: a day at 1,505 operations
of history cost **32 ms and rising**, because application messages chain and the
per-operation cost grows with everything already processed.

*Debug build.* The absolute numbers are pessimistic; the shape is the finding.

## 3. Revocation still cuts forward, and only forward

```
after removal: the reader does not hold the new secret — cut off
and what they already had: still readable
```

Removal rotates the secret, so the next segment names an id the removed reader
has no secret for. What they already hold stays readable, which is what
[D4](../../docs/decisions.md) and §11 promise and never pretend otherwise.

## What this does NOT establish

* **The production DGM and orderer are not ours yet.** `p2panda-spaces` has
  working ones — `EncryptionGroupMembership` and `EncryptionOrderer` — behind
  `pub(crate)`, so this spike uses the crate's `test_utils` implementations
  instead. Shipping D26 means about **242 lines** of our own membership and
  ordering code, or persuading upstream to export theirs. That is bookkeeping
  rather than cryptography, which is the entire argument for the trade, but it
  is not nothing and it is not written.
* **Nothing here replicates, persists or restarts.** No store, no network, no
  reopen. `diaswarm-core`'s vault layout would supply all of that unchanged,
  but that is a claim for the port to prove.
* **The auth layer is untouched.** Who may grant, and the tamper-evident record
  of grants ([D13](../../docs/decisions.md)), is a separate question —
  `p2panda-auth` or the existing signed log.
* **Welcome cost is unmeasured.** D26 names it as the thing that would sink
  this: if handing a joiner the secret bundle is itself linear in history, the
  cost moves rather than disappears. Two members here, one bundle, nothing
  stressed.
