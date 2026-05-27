# Ghost Blockchain - Implementation Summary

## Overview

This summary reflects the implementation currently present in the repository.

## Consensus Architecture

The node runs real Proof-of-Work block authoring via `sc-consensus-pow`. Aura and GRANDPA have been removed from both the node and the runtime. The canonical chain is selected by accumulated PoW (longest-/heaviest-chain); finality is probabilistic, not BFT.

Block authoring is implemented in `node/src/service.rs` and `node/src/pow.rs`:

- Hash function: double-Blake2-256 over `pre_hash || nonce` (64-bit little-endian nonce)
- Difficulty convention: numerically larger `U256` value = harder; a hash (as a big-endian 256-bit integer) is valid iff it is `<= U256::MAX / difficulty`
- Fork choice: `sc-consensus-pow` sums total difficulty for chain selection
- Difficulty value: read from the runtime each block via `sp_consensus_pow::DifficultyApi`, which exposes the value retargeted on-chain by `pallet-ghost-consensus` using timestamp-based retargeting toward a target block time

## Ghost Pallet — Proof-of-Stake Economic Layer

`pallet-ghost-consensus` implements the PoS validator economics layer. It is wired into the live runtime and exposes `DifficultyApi` to the node.

### Pallet lifecycle

1. A miner submits a Ghost header that passes pallet PoW validation.
2. The pallet enters PoS validation for the submitted block.
3. A validator is selected from stakers by stake weight, seeded by the parent hash.
4. The pallet records the validation outcome, distributes rewards, and returns to PoW mode.

### Core pallet capabilities

- Stake and unstake flows with a minimum-stake floor
- Stake-weighted validator selection (seeded by parent hash, stored at submit time)
- Block reward split: 40% to the miner, 60% distributed among stakers; dust handled without loss
- Evidence-gated slashing (`report_misbehavior` requires structured `MisbehaviorEvidence`): double-sign, invalid-block, downtime, and an `Other` proof-hash path; slashed funds are burned
- Validation-timeout recovery: `check_validation_timeout()` returns the pallet to PoW mode if validation stalls past `MaxValidationBlocks`; emits `ValidationTimedOut`
- Bounded state: `MaxValidators` cap on validator membership; `MaxSlashingRecords` cap on slashing history

### Tests

37 pallet unit tests pass, covering staking, validator selection, reward splitting, slashing evidence validation, bounded validator counts, and timeout recovery.

## Post-Quantum Signatures (ML-DSA, NIST FIPS 204)

On-chain ML-DSA signature verification is implemented in `pallets/pallet-ghost-consensus/src/pq_verify.rs` using the pure-Rust `fips204` crate compiled into the no\_std Wasm runtime.

Supported parameter sets: ML-DSA-44 (NIST level 2), ML-DSA-65 (level 3), ML-DSA-87 / Dilithium-5 (level 5).

Pallet extrinsics:

- `register_ml_dsa_key`: validators register an ML-DSA public key on-chain
- `verify_pq_signature`: general-purpose extrinsic for verifying a real ML-DSA signature against a registered key
- `validate_block`: requires a valid ML-DSA signature from the selected validator when that validator has a registered key

Key generation and signing are off-chain operations; the runtime only verifies. Tests include real keygen/sign/verify round-trips and tamper-detection tests.

Ordinary extrinsics still use `MultiSignature` (sr25519/ed25519/ecdsa). ML-DSA is an additional registered validator/attestation path, not a replacement for the account signature scheme.

## Post-Quantum Encryption Module (ML-KEM-1024, NIST FIPS 203)

`node/src/pq_encrypt.rs` implements ML-KEM-1024 key encapsulation (`fips203` crate) combined with ChaCha20-Poly1305 AEAD (`chacha20poly1305` crate) for application-layer payload encryption and operator tooling.

Key lengths (FIPS 203, ML-KEM-1024): encapsulation key 1 568 bytes, decapsulation key 3 168 bytes, ciphertext 1 568 bytes, shared secret 32 bytes.

Security boundary: this module provides post-quantum protection for payloads and operator utilities. It does **not** replace the libp2p Noise/X25519 transport handshake. Node-to-node transport remains classical (X25519 + ChaCha20-Poly1305 Noise) because the `stable2407` polkadot-sdk does not include a PQ-Noise variant.

## Hardening State

### Bounded state

- Validator membership is capped with `MaxValidators`.
- Slashing history uses a bounded vector with `MaxSlashingRecords`.
- Runtime config exposes those limits in `runtime/src/configs/mod.rs`.

### Validation round safety

- Submitted blocks are tracked in `PendingValidationBlock`.
- Validation windows are tracked with `PhaseStartedAt`.
- `check_validation_timeout()` returns to PoW mode if validation stalls.
- `ValidationTimedOut` event is emitted on timeout recovery.

### Evidence-gated slashing

- `report_misbehavior` requires structured `MisbehaviorEvidence`.
- Evidence is validated before stake reduction via `validate_misbehavior_evidence`.
- Slashed funds are burned; evidence attribution is recorded.

## Honesty Boundaries

- Node-to-node transport is classical libp2p with Noise/X25519; it is not post-quantum.
- `MultiSignature` remains the account signature scheme; ML-DSA is an additional validator path.
- Finality is probabilistic longest-chain PoW; there is no BFT finality gadget.
- No external security audit has been performed. The implementation is tested and functional; an audit and multi-node adversarial testing remain before a public mainnet.

## What Remains Before a Production Claim

1. PQ transport: replacing libp2p Noise/X25519 with a PQ-Noise variant
2. External cryptography and integration audit
3. Multi-node adversarial testing under realistic validator-network conditions
4. Production key management, rotation, and operator runbooks
5. Benchmarks and DoS review for PQ signature sizes and verification load under worst-case conditions

## Important Files

- `node/src/service.rs`: PoW node wiring (`sc-consensus-pow`, no Aura/GRANDPA)
- `node/src/pow.rs`: double-Blake2-256 PoW algorithm, `meets_difficulty`, `GhostPow`
- `node/src/pq_encrypt.rs`: ML-KEM-1024 + ChaCha20-Poly1305 encryption module
- `node/src/command.rs`: Ghost CLI status and helper commands
- `runtime/src/configs/mod.rs`: Ghost pallet runtime limits and reward percentages
- `pallets/pallet-ghost-consensus/src/lib.rs`: pallet storage, calls, hooks, and timeout handling
- `pallets/pallet-ghost-consensus/src/pq_verify.rs`: ML-DSA verification (FIPS 204, `no_std` Wasm)
- `pallets/pallet-ghost-consensus/src/functions.rs`: header validation, reward helpers, slashing evidence
- `pallets/pallet-ghost-consensus/src/types.rs`: structured slashing evidence types, `PqAlgorithm` enum
- `pallets/pallet-ghost-consensus/src/tests.rs`: 37 pallet unit tests
