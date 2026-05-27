# Ghost Blockchain — Implementation Progress

Goal: a technically complete, working hybrid PoW + PoS chain with real post-quantum
cryptography. Audits/legal/liquidity are out of scope as launch *gates* only.

## ✅ VERIFIED: a live PoW blockchain that authors & imports blocks

`./target/debug/ghost-node --dev --tmp` boots and produces blocks:
```
🙌 Starting consensus session on top of parent ... (#0)
🎁 Prepared block for proposing at 1
✅ Successfully mined block on top of: ...
🏆 Imported #1 ... #2 ... #3 ... #21    (best: #21 in ~24s)
```
Real Proof-of-Work authoring via `sc-consensus-pow`; the runtime (with on-chain ML-DSA-87,
no Aura/GRANDPA) executes each block. Benign `seal is invalid` lines are multi-thread nonce
races (one thread wins; stale submissions are correctly rejected).

## Phase 1 — Pallet (PoS + PQ signatures) ✅ 37/37 tests pass
Overflow-safe math, conventional PoW difficulty (full U256 vs canonical), real timestamp
difficulty retargeting, slash-and-burn, unstake floor + atomicity, stored validator
selection, slash attribution, header pruning. Real ML-DSA-87 ("Dilithium-5", FIPS 204)
on-chain verification (`fips204`, no_std/Wasm): `register_ml_dsa_key`, `verify_pq_signature`,
mandatory PQ signature in `validate_block` for registered validators.

## Phase 2 — Real PoW node ✅ builds, boots, mines
Removed Aura/GRANDPA from node+runtime; `sp_consensus_pow::DifficultyApi` exposes the pallet
difficulty; `node/src/pow.rs` (GhostPow: double-Blake2 over pre_hash||nonce, U256 target);
`node/src/service.rs` rewrite (PowBlockImport + import_queue + start_mining_worker + OS-thread
CPU miners, longest-chain). Node binary links and runs (see above).

## Phase 3 — PQ encryption (ML-KEM-1024 / Kyber) 🔄 integrating
`node/src/pq_encrypt.rs`: real ML-KEM-1024 (FIPS 203) key encapsulation + ChaCha20-Poly1305
AEAD (`fips203` + `chacha20poly1305`). Final node build verifies it.

## Re-audit fixes applied
HIGH-1 (InvalidBlock evidence uses header's own difficulty, not current → no false slash
after retarget), HIGH-3 (validator signature bound to immutable header fields + block-
specific, no cross-block replay), HIGH-5 (drop LastActiveBlock ghost entries on zero-stake),
MEDIUM-3 (no-staker block → miner gets full reward, nothing dropped), MEDIUM-6 (unstake
returns funds before mutating records).

## Hardening pass 2 (this round)
- Replay guard: `verify_pq_signature` rejects re-submitting an already-recorded
  (attester, statement) attestation (`AttestationAlreadyRecorded`). New test.
- No-staker immediate finality: when the staker set is empty, `submit_block` finalizes the
  PoW block at once (miner gets the full reward) and stays in `PowMining`, instead of
  entering `PosValidation` and waiting for the validation timeout. New test.
- Single PoW source of truth: the `mine` CLI demo now runs the exact node work function
  (`crate::pow::{pow_hash, meets_difficulty}`, conventional difficulty) instead of a
  separate inverted-difficulty hash. CLI `--difficulty` now matches the chain's convention.
- Stale doc fixed: the pallet's module doc no longer claims Aura/GRANDPA authoring.

## Known limitations (honest, not launch-blocking for a devnet)
- Node-to-node transport is classical libp2p Noise/X25519 (stable2407 has no PQ-Noise).
  PQ is signatures (ML-DSA) + an app-layer encryption module (ML-KEM), not a transport swap.
- Block subsidy is inflationary by design (no hard cap); decide a monetary policy before mainnet.
- Weights: `weights.rs` `SubstrateWeight<Runtime>` is wired into the runtime config (replacing the
  `()` placeholder). Each dispatchable's weight is operation-grounded — real `DbWeight` read/write
  counts (incl. staker-set iteration bounded by `MaxValidators`) plus a generous compute allowance,
  with a ~300µs on-chain ML-DSA-87 verify charge on `validate_block`/`verify_pq_signature`. It
  deliberately over-estimates; empirical `frame-benchmarking` numbers are still recommended before mainnet.
- No external security audit; probabilistic (longest-chain) PoW finality, not BFT.

## Build env (Windows): rustc 1.90 stable-msvc; clang/cmake/perl on PATH + LIBCLANG_PATH;
jsonrpsee pinned 0.23.2 (matches stable2407); ahash 0.8.11; getrandom custom backend in runtime.
