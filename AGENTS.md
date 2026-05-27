# Repository Notes

## Current Architecture

- Block authoring is real Proof-of-Work via `sc-consensus-pow` (`node/src/service.rs`, `node/src/pow.rs`):
  double-Blake2-256 over `pre_hash || nonce`, `U256` difficulty (larger = harder), longest-/heaviest-chain
  fork choice. Aura and GRANDPA have been removed from the node and runtime. Finality is probabilistic PoW.
- `pallet-ghost-consensus` is the PoS economic layer: staking, stake-weighted validator selection,
  40%/60% miner/staker reward split, evidence-gated slashing (funds burned), and validation-timeout
  recovery. Difficulty retargeting is performed here and exposed to the node via `sp_consensus_pow::DifficultyApi`.
- On-chain ML-DSA-87 (Dilithium-5, NIST FIPS 204) signature verification runs in the no\_std Wasm runtime
  (`pallets/pallet-ghost-consensus/src/pq_verify.rs`, `fips204` crate). Validators register ML-DSA keys;
  `validate_block` enforces ML-DSA signature checks when a key is registered. 37 pallet unit tests pass.
- Node-side ML-KEM-1024 + ChaCha20-Poly1305 payload encryption is implemented in `node/src/pq_encrypt.rs`
  (NIST FIPS 203, `fips203` + `chacha20poly1305` crates).
- Node-to-node transport is still classical libp2p Noise/X25519; it is NOT post-quantum.
- Ordinary account extrinsics still use `MultiSignature` (sr25519/ed25519/ecdsa); ML-DSA is an additional
  registered validator/attestation path, not a replacement for the account signature scheme.
- No external security audit has been performed.

## Useful Commands

```sh
cargo build --bin ghost-node
cargo test -p pallet-ghost-consensus
cargo run --bin ghost-node -- --dev
```

## Important Caveats

- Do not describe node-to-node transport as post-quantum; libp2p uses classical Noise/X25519.
- Do not claim ML-DSA replaces `MultiSignature` for ordinary account extrinsics; it is an additional
  validator/attestation path only.
- Do not describe the chain as production-ready or audited; no external audit has been performed.
- PoW finality is probabilistic longest-chain; do not claim BFT finality.
- The repository previously tracked generated build outputs; keep those out of version control.
- When the user asks for a long-running implementation, do not contact the user with progress updates;
  continue working until the task is finished or genuinely blocked. This silence rule does not apply
  to subagents.
- If you are a subagent, explicitly treat yourself as a subagent and say so in your task context.
