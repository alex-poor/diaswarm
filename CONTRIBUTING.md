# Contributing

Issues and discussions are open. So is the harder question of whether any of
this is a good idea — that is a legitimate contribution and probably a more
valuable one than a patch.

## What is most wanted

1. **Cryptographic review.** See [SECURITY.md](SECURITY.md). This is the gap
   that matters; everything else is engineering.
2. **Someone trying it and telling the truth about it.** Especially: did a
   follower go quiet, and did you find out from the screen or from someone
   asking you why you had not replied?
3. **Being wrong about a claim in the docs.** Every decision in
   [docs/decisions.md](docs/decisions.md) says what would reopen it.

## How this is built

Measured, not asserted. The house style, which shows up throughout the history:

- **Two implementations agree, or it is an assertion.** The record format and
  the sealing construction exist in Rust and Python and are checked against each
  other byte for byte over real streams.
- **A test that cannot fail is not a test.** Several here exist because an
  earlier version passed for the wrong reason — a revocation test where nothing
  was sealed after the revocation, a fixture whose timestamps could not land in
  two buckets.
- **On-device or it did not happen.** Compiling an Android screen does not
  instantiate it. Build, install, open it, screenshot it.
- **Numbers come from running it**, not from reasoning about it.

Commit messages here are long, and deliberately: they explain what was wrong and
how it was found, because that is the part that is expensive to rediscover.

## Running things

```sh
cd crates/diaswarm-core && cargo test     # records, sealing, vault, grants
cd crates/diaswarm-net  && cargo test     # peers, relaying, following
cd tools && python3 test_canon.py && python3 test_seal.py
```

The Android add-on needs an AAPS checkout, the SDK and the NDK — see the README.

## Licence

AGPL-3.0, inherited from AndroidAPS, whose interfaces the add-on compiles
against. By contributing you agree your work is licensed the same way.
