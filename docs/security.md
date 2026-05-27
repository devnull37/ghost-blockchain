# Security Notes

## Scope

This document describes the current security posture of the Ghost blockchain and the minimum requirements for any
future work on the cryptographic stack.

### What is implemented

- Real Proof-of-Work block authoring via `sc-consensus-pow` (double-Blake2-256, `U256` difficulty). Aura and GRANDPA
  have been removed. Finality is probabilistic longest-chain PoW, not BFT.
- On-chain ML-DSA signature verification (NIST FIPS 204, pure-Rust `fips204` crate, `no_std` Wasm runtime).
  Validators may register ML-DSA public keys; `validate_block` enforces ML-DSA signature checks when a key is
  registered. All three ML-DSA parameter sets (ML-DSA-44, ML-DSA-65, ML-DSA-87) are supported.
- Node-side ML-KEM-1024 key encapsulation + ChaCha20-Poly1305 AEAD (NIST FIPS 203) for application-layer payload
  encryption (`node/src/pq_encrypt.rs`).

### What is NOT post-quantum

- **Node-to-node transport is classical.** The node uses libp2p with a Noise/X25519 handshake. The `stable2407`
  polkadot-sdk does not include a PQ-Noise variant. Network transport is not post-quantum.
- **Ordinary account signatures use `MultiSignature`** (sr25519/ed25519/ecdsa). ML-DSA is an additional
  registered validator/attestation path, not a replacement for the account signature scheme.

### What has not been audited

No external security audit of this codebase has been performed. The implementation is tested and functional,
but an audit and multi-node adversarial testing remain before any production mainnet claim is appropriate.

## Local prerequisite checklist

Before starting crypto-adjacent work, confirm that you can run the normal local build and test path:

```bash
cargo build --bin ghost-node
cargo test -p pallet-ghost-consensus
```

You should also record the active toolchain and crypto-related host dependencies:

```bash
rustup show
rustc --version
cargo --version
openssl version
perl -v
```

On native Windows, `perl -v` matters because this repository has already hit an `openssl-sys` failure where vendored
OpenSSL required Perl to continue building.

## Windows-specific caution

WSL is the preferred development environment for this codebase when working near TLS, libssl, or PQ dependency
evaluation.

If native Windows is unavoidable, treat the following as mandatory baseline dependencies:

- Visual Studio Build Tools with C++
- a Perl distribution on `PATH`
- OpenSSL headers and libraries, or a working vendored OpenSSL build path

Do not treat a successful pallet unit test run as proof that the node's native crypto dependencies are healthy.
The full node build should pass too.

## Audit requirements for PQ or network crypto changes

No PQ or network-crypto implementation should be described as complete, secure, or production-ready until it has
passed all of the following:

1. A written threat model covering peer transport, key material, downgrade paths, replay behavior, and failure modes.
2. A dependency review for every new cryptographic crate, C library, or FFI boundary.
3. Reproducible local builds on the supported developer platforms, including at least one Unix-like environment.
4. Negative tests for malformed inputs, handshake failures, and algorithm-mismatch scenarios.
5. Secret-handling review for key generation, storage, rotation, logging, and crash output.
6. An external security audit by reviewers with cryptography and systems experience before any production claim.

## Minimum evidence expected in a change

Any serious PQ or network-crypto pull request should include:

- the exact algorithms and libraries being evaluated
- why those choices fit the threat model
- platform-specific build notes, especially for Windows and OpenSSL-linked dependencies
- test coverage for success and failure paths
- a clear statement of what remains experimental or unaudited

## Claim gates

Use the following minimum gates before updating documentation language:

- "PQ validator signatures" — ML-DSA verification is implemented and tested in the runtime. This gate is met for
  the registered-validator path. It does not extend to the account `MultiSignature` scheme.
- "PQ payload encryption" — ML-KEM-1024 + ChaCha20-Poly1305 is implemented in `node/src/pq_encrypt.rs` for
  application-layer use. This gate is met for operator/payload use cases.
- "PQ transport" — not met. Requires a PQ-Noise libp2p variant, an implemented transport design,
  interoperability testing, and audit coverage for the actual network path.
- "PQ consensus" — not met. Finality is probabilistic PoW. Replacing Aura/GRANDPA with classical PoW does not
  make consensus post-quantum; PQ consensus would require that all critical consensus signatures use PQ schemes.
- "Production ready" — not met until an external audit is complete, multi-node adversarial testing passes, and
  all audit findings are triaged and resolved.

If a gate is not met, do not describe the feature as complete for that scope.
