# Ghost Blockchain

Ghost is a Substrate-based chain with a hybrid Proof-of-Work / Proof-of-Stake consensus engine and on-chain post-quantum signature verification.

## Current State

- Block authoring is real Proof-of-Work via `sc-consensus-pow` (`node/src/service.rs`, `node/src/pow.rs`). Aura and GRANDPA have been removed from both the node and runtime. Finality is probabilistic longest-/heaviest-chain PoW; there is no BFT finality gadget.
- `pallet-ghost-consensus` implements the PoS economic layer: staking/unstaking with a minimum-stake floor, stake-weighted validator selection, a 40%/60% miner/staker reward split, evidence-gated slashing that burns funds, and validation-timeout recovery. 37 pallet unit tests pass.
- On-chain ML-DSA (NIST FIPS 204) signature verification is implemented and tested in the runtime Wasm (`pallets/pallet-ghost-consensus/src/pq_verify.rs`, `fips204` crate). Validators may register an ML-DSA public key; `validate_block` requires a valid ML-DSA signature from the selected validator when a key is registered.
- Node-side ML-KEM-1024 + ChaCha20-Poly1305 payload encryption is implemented (`node/src/pq_encrypt.rs`, NIST FIPS 203).
- No external security audit has been performed. The implementation is tested and functional; an audit and multi-node adversarial testing remain before a public mainnet.

## What Works

- Real PoW block production and import via `sc-consensus-pow` with double-Blake2-256 (`pre_hash || nonce`), `U256` difficulty, and difficulty retargeting in `pallet-ghost-consensus`
- Pallet-level staking, validator selection, reward distribution, and slashing logic with 37 passing unit tests
- On-chain ML-DSA-87 (Dilithium-5, FIPS 204) signature verification inside the no\_std Wasm runtime
- Node-side ML-KEM-1024 key encapsulation + ChaCha20-Poly1305 AEAD for application-layer payload encryption
- Runtime guardrails: bounded validator count, bounded slashing history, evidence-gated slashing, validation timeout recovery

## Honesty Boundaries

- Node-to-node transport is still classical: libp2p with Noise/X25519. The `stable2407` polkadot-sdk does not include a PQ-Noise variant, so **the transport layer is not post-quantum**.
- Ordinary extrinsics still use `MultiSignature` (sr25519/ed25519/ecdsa). ML-DSA is an additional registered validator/attestation path, not a replacement for the account signature scheme.
- PoW finality is probabilistic (longest-chain), not BFT.
- No external security audit has been completed. Do not describe this as production-ready for a public mainnet.

## What Remains

- PQ transport (replacing libp2p Noise/X25519 with a PQ-Noise variant)
- External cryptography and integration audit
- Multi-node adversarial testing
- Production key management tooling and operator runbooks

See `docs/pqc-roadmap.md` for the staged implementation plan with current completion status.

## Build

```sh
cargo build --bin ghost-node
```

## Test

```sh
cargo test -p pallet-ghost-consensus --target-dir .cargo-target
```

## Run A Local Node

```sh
cargo run --bin ghost-node -- --dev
```

## Ghost CLI Helpers

```sh
cargo run --bin ghost-node -- ghost status --detailed
cargo run --bin ghost-node -- ghost mine --threads 2
```

## Important Paths

- `node/src/service.rs`: PoW node wiring (`sc-consensus-pow`, no Aura/GRANDPA)
- `node/src/pow.rs`: double-Blake2-256 PoW algorithm and difficulty check
- `node/src/pq_encrypt.rs`: ML-KEM-1024 + ChaCha20-Poly1305 encryption module
- `node/src/command.rs`: Ghost helper CLI
- `runtime/src/lib.rs`: runtime composition
- `runtime/src/configs/mod.rs`: runtime pallet configs and limits
- `pallets/pallet-ghost-consensus/src/lib.rs`: pallet storage, calls, hooks, and timeout handling
- `pallets/pallet-ghost-consensus/src/pq_verify.rs`: ML-DSA verification (FIPS 204, `no_std` Wasm)
- `pallets/pallet-ghost-consensus/src/functions.rs`: header validation, reward helpers, slashing evidence
- `pallets/pallet-ghost-consensus/src/tests.rs`: 37 pallet unit tests
