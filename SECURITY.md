# Security

## The state of this, plainly

**The cryptography here has not been reviewed by anyone qualified.** It is
standard primitives — X25519, HKDF-SHA256, ChaCha20-Poly1305, Ed25519 — composed
by hand into a scheme nobody has audited. It is tested, it is documented, and
two independent implementations agree byte for byte. None of that is a review.

This project is about diabetes data. Being wrong about who can read it is a real
harm to real people, so nothing here should be described as secure until
somebody who does this for a living has said so.

**If you are deciding whether to trust this with your data today: don't, unless
you are willing to be the person who finds the problem.**

## Reporting something

**Please report privately first.** Use GitHub's
[private vulnerability reporting](https://github.com/alex-poor/diaswarm/security/advisories/new)
on this repository. If that is not available to you, open an issue saying only
that you have found something and asking for a contact — no details in the
issue.

There is no bounty, no SLA and no team. There is one person who will read it and
take it seriously.

## What would count as a finding

The design makes specific claims. Any of these being false is a real finding:

- **A holder can read what it holds.** Peers store sealed segments, every
  reader's wraps, and the grant log. None of it should open without a granted
  key.
- **A revoked reader gains anything new.** Withdrawal rotates the segment; from
  that point they should be wrapped for nothing.
- **A grant log can be altered undetectably** by anyone, including the subject,
  in a copy someone else also holds.
- **The log identifies a reader.** Tags derive from a shared secret; the same
  reader should be an unrelated tag to every subject.
- **A fetch reveals who is fetching.** Every peer sends the identical request.
- **Anything in the record stream leaks more than intended** — the canonical
  form is meant to carry treatment data, not identifiers.

## Known and accepted, not bugs

These are consequences of the design, written down so nobody has to rediscover
them as surprises. They are discussed in
[docs/feasibility.md](docs/feasibility.md) and [docs/decisions.md](docs/decisions.md).

- **No forward secrecy.** A leaked key opens everything it was ever wrapped for,
  for ever. There is no expiry.
- **Publication is permanent.** There is no delete and no recall. Revocation is
  prospective only — it stops the next segment, and cannot touch a copy someone
  already has.
- **Reads are invisible.** Nobody can learn who read their data, or when. This
  is deliberate and it cuts both ways.
- **Metadata leaks.** How many grants exist and roughly when they happened is
  visible to any holder, even though who they name is not.
- **An invite is public.** It carries a public key and an endpoint. Anyone
  holding one can download ciphertext and open none of it.
- **The social graph.** Who talks to whom over the network is not hidden.

## What this cannot do to a pump

The AAPS add-on is a `DataSyncSelector`: it drains a queue outward. It holds no
pump reference, implements no constraint, and has no path into dosing. It ships
disabled.

The realistic risk is not dosing — it is availability. A plugin that fails to
construct takes the app down with it, and an app that will not start is a loop
that has stopped. **A crash on a looping phone is a safety issue and should be
reported as one**, even though nothing here can deliver insulin.
