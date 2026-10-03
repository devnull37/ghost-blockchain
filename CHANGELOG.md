# Changelog

All notable changes to the Ghost chain are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). `spec_version`
tracks the runtime; node versions tag independently.

## Unreleased — spec_version 102

### Consensus engine (replaced Aura with real Ghost PoW)
- `ghost-consensus` crate: `GhostPowAlgorithm` (work-factor PoW), `HeaviestChain` fork choice, deterministic tie-break on seal `pre_hash`, mining grind helpers, `PowAux` persistence.
- `ghost-pow-primitives`: `GhostSeal`, `POW_ENGINE_ID`, `GhostPowApi::next_difficulty`, shared `compute_next_difficulty` (fuzz-proven identical between node and pallet).
- Node service: `sc-consensus-pow` import queue + mining worker on `--mine`, GRANDPA over the heaviest chain, `offchain_tx_pool_factory` for equivocation reports. Aura removed entirely.

### Runtime
- `pallet-ghost-consensus` v2: holds-based staking, deterministic top-N stake `SessionManager` with M-4 fallbacks, difficulty retarget every 100 blocks, digest-decoded 40/60 rewards, `OnOffenceHandler` slashing (incl. unbonding chunks), bounded storage throughout.
- `pallet-ghost-pqc` (index 14): ML-DSA-87 (FIPS-204) registry, PoP registration, `pqc_attest` gated to bonded validators, `RequirePqcKey` validator gating.
- `pallet-template` removed; `spec_version` 101 → 102.
- `MinimumPeriod = 1` (timestamp ratchet off — soak-validated fix for rejected honest seals).
- Session committee + equivocation + im-online wiring; `PowFindAuthor` feeds real authors to im-online.

### Toolchain & CI
- `rust-toolchain.toml` pins 1.88.0 + wasm32.
- CI: fmt, clippy, test, wasm-build, node build, e2e, e2e-faults, cargo audit, cargo deny; scheduled weekly soak.
- Gates: `e2e-ghost.sh` (2-miner PoW smoke), `e2e-faults.sh` (miner halt / committee offline / equivocation→slash), `soak-ghost.sh` (multi-node 40 min default).

### Security
- Round-1 review: 19 findings; C-1 (unwired v1→v2 migration), H-1 (unbond-immunity), H-2 (im-online author starvation) fixed.
- Round-2 review: post-round-1 deltas; R2-1/R2-2 fixed in place; 4 documented residuals.
- Fuzz: 4 cargo-fuzz targets, ~17M execs, found the retarget overflow divergence (fixed via shared math).

### Docs
- Protocol spec, threat model, economic parameters, operator guide, node-ops, RPC surface audit, runtime-upgrade policy, testnet-readiness, soak report, CONTRIBUTING.
