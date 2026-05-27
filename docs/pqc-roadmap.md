# Post-Quantum Crypto Integration Roadmap

## Scope and current reality

This document tracks the staged post-quantum integration for the Ghost blockchain. Stages that are
complete are marked **[DONE]**. Stages that remain are marked **[TODO]**.

### What is implemented today

- **Real PoW consensus**: `sc-consensus-pow` with double-Blake2-256 and on-chain difficulty retargeting.
  Aura and GRANDPA have been removed from the node and runtime.
- **ML-DSA on-chain verification [DONE]**: ML-DSA-87 (Dilithium-5, NIST FIPS 204, security level 5) signature
  verification runs inside the no\_std Wasm runtime via the pure-Rust `fips204` crate
  (`pallets/pallet-ghost-consensus/src/pq_verify.rs`). Validators register ML-DSA public keys;
  `validate_block` enforces ML-DSA signature checks when a key is registered; the general
  `verify_pq_signature` extrinsic is available. Tested with real keygen/sign/verify and tamper tests.
- **ML-KEM-1024 node-side encryption [DONE]**: ML-KEM-1024 key encapsulation (NIST FIPS 203, security level 5)
  + ChaCha20-Poly1305 AEAD is implemented for application-layer payload encryption and operator tooling
  (`node/src/pq_encrypt.rs`, `fips203` + `chacha20poly1305` crates).

### What is NOT post-quantum

- **Node-to-node transport is classical.** libp2p uses a Noise/X25519 handshake. The `stable2407`
  polkadot-sdk does not include a PQ-Noise variant. Network transport is not post-quantum.
- **Account signature scheme** still uses `MultiSignature` (sr25519/ed25519/ecdsa). ML-DSA is an
  additional registered validator/attestation path, not a replacement for account signing.
- **No external audit** has been performed; the implementation is tested and functional but not
  production-certified.

## Standards baseline

All implemented and planned work uses standardized primitives:

- `ML-DSA` (NIST FIPS 204, August 13 2024) — post-quantum digital signatures; **implemented on-chain**
- `ML-KEM` (NIST FIPS 203, August 13 2024) — key encapsulation; **implemented node-side**
- `SLH-DSA` (NIST FIPS 205, August 13 2024) — conservative hash-based signature fallback; **not yet implemented**

## Staged implementation plan

### Phase A: PQ validator signatures and payload encryption — [DONE]

- ML-DSA-87 on-chain verification in the Wasm runtime.
- Validator ML-DSA key registration extrinsic.
- `validate_block` enforces ML-DSA signature when a key is registered.
- ML-KEM-1024 + ChaCha20-Poly1305 node-side payload encryption.
- Real sign/verify/tamper test coverage.

What this phase does NOT provide: PQ account transaction signing; PQ transport; BFT PQ finality.

### Phase B: Hybrid validator identity and account PQ signing — [TODO]

- Extend or replace `MultiSignature` in `runtime/src/lib.rs` to support PQ-signed extrinsics.
- Add SCALE encoding, extrinsic verification, benchmarks, and weight updates for PQ signatures.
- Prove key lifecycle: generation, rotation, revocation, and recovery for ML-DSA validator keys.
- Add monitoring for key mismatch, stale attestations, and signer failure.
- Benchmark: extrinsic size limits, block weight, transaction pool memory, and RPC payload sizes.

Exit criteria: PQ-signed extrinsics execute in tests; malformed signatures fail deterministically;
block size and throughput impact are measured and documented.

### Phase C: PQ transport — [TODO]

- Replace libp2p Noise/X25519 with a PQ-Noise variant (requires polkadot-sdk / libp2p upgrade or
  custom transport integration).
- Interoperability tests with classical peers during any transition period.
- Threat model covering downgrade paths, handshake failures, and replay behavior.

Exit criteria: node-to-node connections are post-quantum hardened; classical fallback is either
disabled or explicitly controlled and documented.

### Phase D: External audit and release gating — [TODO]

- Internal cryptography review.
- Independent external cryptography audit covering ML-DSA and ML-KEM usage, runtime verification
  boundaries, and key management.
- Independent protocol/integration audit.
- Fuzzing and property testing for signature parsing and verification boundaries.
- Performance and DoS review for oversized payloads and verification storms.
- Release checklist: rollback procedures, kill-switches, chain-upgrade safety.

Exit criteria: all audit findings triaged and closed or explicitly accepted; production release notes
contain no unsupported PQ claims.

## Concrete implementation roadmap for this repo

### Stage 0: Threat model and success criteria — [DONE for implemented scope]

Completed:

- ML-DSA chosen as the primary PQ signature family (FIPS 204); ML-KEM-1024 for key encapsulation (FIPS 203).
- Target scope defined: validator/attestation signatures + application-layer payload encryption; account
  transaction signing and transport are explicitly out of scope for the current phase.
- Runtime Wasm build compatibility confirmed (`fips204` in `no_std`).

Remaining:

- Written threat model document for Phase C transport work.
- Approved rollout and rollback criteria for Phase B account signing changes.

### Stage 1: Crypto abstraction layer — [DONE]

Completed:

- `pq_verify.rs` internal module isolates all `fips204` API usage behind `verify_ml_dsa` / `validate_ml_dsa_pk`.
- Known-answer tests (real keygen/sign/verify) for ML-DSA-87 pass.
- `no_std` and Wasm build compatibility confirmed.
- Deterministic, panic-free verification with no RNG dependency in the runtime code path.

Remaining:

- Benchmark harness for ML-DSA verification cost at block limits.
- Equivalent abstraction module for Phase B account signing work.

### Stage 2: PQ extrinsic support — [PARTIAL]

Completed:

- `verify_pq_signature` extrinsic verifies real ML-DSA signatures on-chain.
- `validate_block` enforces ML-DSA signature from the selected validator when a key is registered.
- Negative tests for invalid signatures are present.

Remaining:

- Extend or replace `MultiSignature` in `runtime/src/lib.rs` for general account PQ signing.
- Update transaction submission tooling and any tooling that assumes sr25519/ed25519/ecdsa.
- Benchmark verification cost; update weights and block limits accordingly.
- Add negative tests for malformed, truncated, replayed, and oversized signatures in the extrinsic path.

### Stage 3: Tooling and operations — [PARTIAL]

Completed:

- Ghost CLI helpers for the miner and basic chain status.
- ML-KEM-1024 encryption utility in `node/src/pq_encrypt.rs`.

Remaining:

- Key generation/import/export tooling for ML-DSA scheme.
- Chain-spec and genesis support for PQ validator key material.
- RPC compatibility tests for author submission and account queries with PQ keys.
- Operator runbooks for rotation, backup, recovery, and incident response.

### Stage 4: Hybrid validator migration — [PARTIAL]

Completed:

- Validator ML-DSA key registration and enforcement in `validate_block` is the functional core of hybrid
  operation: validators can operate with both a classical account key and a registered ML-DSA key.

Remaining:

- Monitoring for key mismatch, stale attestations, and signer failure.
- Incident drills for lost or rotated ML-DSA keys.
- Operational evidence that dual-key validator workflows run reliably over extended periods.

### Stage 5: PQ transport — [TODO]

Primary code targets:

- libp2p transport configuration (requires polkadot-sdk upgrade or custom integration)
- `node/src/service.rs` network configuration hooks

Exit criteria: live block production no longer requires a classical Noise/X25519 handshake;
validator-node interoperability tests pass across restart, equivocation, and partition scenarios.

### Stage 6: Audit and release gating — [TODO]

See Phase D above.

## Audit checklist

An audit firm should be able to answer "yes" to all of these before any production claim:

- Are all validator signature checks enforced using the new ML-DSA design, not only pallet-local messages?
- Are transaction signatures (if extended to account scheme) verified by standardized PQ algorithms?
- Are all runtime verifiers deterministic under Wasm?
- Are signature parsing and length checks hardened against panic and memory abuse?
- Are weight limits and block limits updated for worst-case PQ verification load?
- Are keystore, RPC, and validator workflows covered end to end?
- Are downgrade paths to classical-only behavior either disabled or explicitly controlled?
- Are wallet and operator UX paths resistant to accidental misuse?
- Are all public docs accurate about what remains classical?

## What this repo can honestly claim at each milestone

- **Now (Stage 1 + Stage 2 partial + Stage 4 partial)**: "the runtime verifies ML-DSA validator signatures
  in the Wasm runtime; ML-KEM-1024 payload encryption is available node-side; ordinary account transactions
  and network transport remain classical."
- After Stage 2 complete: "the runtime can verify PQ-signed extrinsics for all account types in tests."
- After Stage 4 complete: "validators operate hybrid classical plus ML-DSA identity workflows with
  production key lifecycle tooling."
- After Stage 5 and Stage 6: "live consensus signatures, transaction signing, and node-to-node transport
  have completed the PQ migration."

## Documentation gate for future claims

Do not update repository docs to claim transport-level or full-consensus PQ unless the relevant stage is
complete:

- ML-DSA validator signature claims: Stage 1 + Stage 2 partial — **met for the validator path**.
- ML-KEM payload encryption claims: Stage 1 — **met for node-side use**.
- PQ account transaction-signing claims: require Stage 2 complete plus measured verification costs.
- PQ transport claims: require Stage 5 complete plus transport design, dependency review,
  interoperability tests, and audit coverage.
- Production-ready claims: require Stage 6 complete.

Describe any work outside these completed stages as in-progress or planned.
