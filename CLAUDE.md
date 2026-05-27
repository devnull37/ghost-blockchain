# Repository Notes

## Current Architecture

- Block authoring is real Proof-of-Work via `sc-consensus-pow` (`node/src/service.rs`, `node/src/pow.rs`):
  double-Blake2-256 over `pre_hash || nonce`, `U256` difficulty, longest-/heaviest-chain fork choice.
  Aura and GRANDPA have been removed from the node and runtime. Finality is probabilistic PoW, not BFT.
- `pallet-ghost-consensus` is the PoS economic layer: staking/unstaking, stake-weighted validator
  selection, reward splitting (40% miner / 60% stakers), evidence-gated slashing (funds burned), and
  validation-timeout recovery. Exposes `DifficultyApi` to the node.
- On-chain ML-DSA-87 (NIST FIPS 204) signature verification is implemented in the Wasm runtime
  (`pq_verify.rs`, `fips204` crate). Validators register ML-DSA keys; `validate_block` enforces ML-DSA
  checks when a key is registered. 37 pallet unit tests pass.
- Node-side ML-KEM-1024 + ChaCha20-Poly1305 payload encryption is implemented in `pq_encrypt.rs`
  (NIST FIPS 203).
- Node-to-node transport is classical libp2p Noise/X25519; it is NOT post-quantum.
- Ordinary account extrinsics still use `MultiSignature`; ML-DSA is an additional validator/attestation
  path only. No external security audit has been performed.

## Useful Commands

```sh
cargo build --bin ghost-node
cargo test -p pallet-ghost-consensus
cargo run --bin ghost-node -- --dev
```

## Important Caveats

- Do not describe node-to-node transport as post-quantum; libp2p uses classical Noise/X25519.
- Do not claim ML-DSA replaces `MultiSignature` for ordinary account extrinsics; it is an additional
  validator/attestation path.
- Do not describe the chain as production-ready or audited; no external audit has been performed.
- PoW finality is probabilistic longest-chain; do not claim BFT finality.
- The repository previously tracked generated build outputs; keep those out of version control.
