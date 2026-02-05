# Ghost Chain: Quantum-Resistant Hybrid Consensus

## The "2" Strategy: Quantum Hardening
We are integrating Post-Quantum Cryptography (PQC) into the Substrate pallet to make Ghost Chain "Future-Proof".

### ⚖️ Hybrid PQC-Classical Signature Scheme
- **Primary:** Crystals-Dilithium (NIST Round 3 Winner). Used for block finalization by the PoS committee.
- **Security:** Level 5 (equivalent to AES-256).
- **PoW synergy:** The PoW puzzle now includes a "PQC hash-check" where miners must solve a lattice-based challenge, making them partially resistant to future quantum-accelerated hashing (Grover's algorithm).

### 🛠️ Substrate Implementation
- Added `pqc_logic.rs` to the workspace.
- The committee signature verification will move from Ed25519 to Dilithium in the `finalize_block` phase.

## Current Progress
1.  [x] Drafted PQC signature wrapper.
2.  [ ] Integrate lattice-based difficulty adjustment into PoW logic.
3.  [ ] Benchmarking PQC signature verification in WASM.
