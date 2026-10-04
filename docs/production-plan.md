# Ghost Production Plan

Release bar: the standard a top-tier organization would apply before shipping a public network. Not "it compiles and a demo passes" — provable consensus, provable economics, reproducible ops, and a security posture that survives adversarial review. Every item below is an acceptance-gated checkbox; nothing is claimed until demonstrated.

## P0 — Blocking for any testnet claim

### Consensus core (must be real, not simulated)
- [x] PoW block production live in the node service (`sc-consensus-pow`), Aura fully removed — PR #13 (mining verified: `--dev --mine` seals + imports, no-`--mine` nodes import-only)
- [ ] Seal verification on import at every node; invalid seals rejected deterministically across nodes (proven by e2e with a forged-seal block attempt) — unit-level done (PR #9 verify tests, PR #14 embedded-pre_hash check); e2e forged-block attempt pending
- [x] Heaviest-chain fork choice via total difficulty; tie-break = smallest seal `pre_hash`, applied identically by `break_tie` and `HeaviestChain` — PR #9 + commit fd80f3b (M-1)
- [x] GRANDPA finality over the PoW chain — PR #13 (finalized head advances under `--dev --mine`; 2-miner contention proven by e2e, pending)
- [x] Stake-driven validator committee via `pallet_session`; top-N stake selection — PR #10
- [x] Difficulty retargeting deterministic on-chain; unit + proptest bounds — PR #10, ghost-consensus difficulty tests
- [x] Miner attribution unspoofable: author decoded from seal-bound `PreRuntime(pow_, AccountId32)`; no extrinsic reward claim — PR #9/#10/#13
- [x] Rewards: 40% miner / 60% active validators minted on-chain — PR #10; issuance-conservation test present
- [x] Staking: bond/bond_extra/unbond/withdraw_unbonded (holds) + validate/chill + MinStake + bounded unbonding chunks — PR #10
- [x] Slashing: GRANDPA equivocation → offences → slash + chill incl. unbonding chunks (no unbond-immunity) — PR #10 + #14 (H-1); downtime heartbeats via im-online + PowFindAuthor (H-2)
- [x] All consensus-relevant storage bounded; no unbounded iteration in any hot path — round-1 cleared the pallet; round-2 signed off all post-round-1 deltas (docs/security-review/round-2.md)
- [x] Real weights: benchmarks for all 10 extrinsics + checked-in `weights.rs` — PR #15 (regenerate via node CLI pending)

### Correctness & safety engineering
- [x] `cargo test --workspace` green; `cargo fmt --check`; `cargo clippy -- -D warnings` zero warnings — CI green on PR #8; kept green through PR #13/#15
- [x] Property tests (proptest): retarget bounds/monotonicity, seal verify accept/reject vs mining — PR #9/#10 (reward-conservation + selection-distribution properties in pallet tests)
- [x] Fuzz target for seal decode + digest parsing (bounded input, no panic on arbitrary bytes) — PR #18 (4 cargo-fuzz targets ~17M execs zero crashes + found real retarget-math divergence, fixed via shared `compute_next_difficulty`)
- [x] e2e on the Ghost path (`scripts/e2e-ghost.sh`): 2 miners + committee peer, author PoW blocks (seal digests via RPC), finalize, rewards land, restart+rejoin — PR #17 (7-check gate, CI e2e job repointed at it)
- [x] Soak (`scripts/soak-ghost.sh`): PASSED 2400s on spec-103 release — 797 blocks, fin lag avg 2.3/max 12, both keyless SIGKILL restarts resynced+resumed authoring, 0 TooFarInFuture, 0 violations — docs/soak-report.md
- [x] Failure-mode tests: `scripts/e2e-faults.sh` PASSED on spec-103 — scenario A miner halt (finality caught the frozen best at the fin=best−2 voting-rule equilibrium, held without overshoot, resumed on return), B committee member offline (best advanced, finality pinned at 0, resumed on rejoin), C equivocation (crafted GRANDPA-signed prevote pair submitted via signed `report_equivocation`; SlashRecords names Alice, bond 1e15→0, liveness kept) — docs/adversarial-report.md

### Ops baseline
- [x] CI on every PR: fmt, clippy, tests, node build, e2e-smoke — PR #8 (6 jobs green)
- [x] Dockerfile builds `ghost-node` + compose multi-node — PR #8 (docker build + compose verified)
- [x] `ghost-node --version`, `--help`, `build-spec`, `export-chain-spec` work — verified during service merge
- [ ] docs/testnet-readiness.md reflects the true gate — PR #12 updated sections; final pass after e2e/soak land

## P1 — Required before public testnet launch

### Security & economics hardening
- [x] Written threat model (docs/threat-model.md): adversary classes, per-attack mitigations vs residual risk, known-gaps register — PR #12
- [x] Economic parameter doc: docs/economic-parameters.md — every constant named to its runtime definition, reward split math, liveness failure semantics
- [x] `cargo deny`/`cargo audit` in CI: `audit` (rustsec/audit-check) + `deny` (licenses/sources hard gate, bans warn) jobs landed. `unsafe` remains forbidden
- [x] External-style security review: round-1 (PR #14, 19 findings, C-1/H-1/H-2 fixed) + round-2 (docs/security-review/round-2.md — R2-1/R2-2 fixed in place, 4 accepted residuals documented)
- [x] Session key ops documented: docs/operator-guide.md — rotateKeys/set_keys, NextKeys boundary timing, PQC register/rotate flow, equivocation warning, slashing table
- [x] RPC surface audit: docs/rpc-surface.md — full method inventory, DenyUnsafe plumbing verified, `--rpc-methods` guidance

### Runtime quality
- [x] `spec_version`/`impl_version`/`transaction_version` bump policy documented (docs/runtime-upgrade-policy.md); `scripts/e2e-upgrade.sh` PASSED on spec-103 — real `sudo(system.set_code)` applied spec 103→104 on a live chain, finality continued, GhostConsensus storage version intact; drill found+fixed the polkadot-js u8a-Bytes empty-`:code` hazard (documented in the policy)
- [x] Storage migration framework convention (versioned storage, `OnRuntimeUpgrade` hooks) + a tested no-op migration — MigrateToV2 now wired via `Hooks::on_runtime_upgrade` (was tested-but-unreachable); `migration_is_noop_when_already_v2` green; ghost-pqc stamps storage_version(1)
- [x] Genesis ceremony doc for public testnet: docs/genesis-ceremony.md + `scripts/genesis-generate.sh` + `chainspecs/ghost-testnet-params.example.json` — spec generation verified end-to-end (plain+raw+SHA256); real allocation table is ceremony-time material by design
- [x] SS58 prefix decision documented (docs/genesis-ceremony.md — default 42 acceptable for testnet, registration required before mainnet)
- [ ] Remove `pallet-template` and `sudo` from production preset: pallet-template fully removed from the runtime (crate deleted, spec_version 103). sudo removal is a chainspec-preset decision — tracked under the genesis ceremony item

### Ops maturity
- [x] Telemetry/metrics documented (docs/node-ops.md — alert table, best-vs-finalized diagnostics; honest note: no dedicated hashrate metric yet)
- [x] Node ops docs (docs/node-ops.md): sizing, pruning, metrics, telemetry, upgrades, keystore/backup, incident runbook
- [x] Release pipeline: `.github/workflows/release.yml` on `v*` tags → release binary + SHA256SUMS + bundled chainspec + generated notes (Docker image pending a registry credential — Dockerfile builds standalone)
- [x] Polkadot.js Apps compatibility verified at the RPC layer: full legacy + spec-v2 method set (102), complete metadata blob, `system_properties` now serves ss58Format/decimals/symbol (was `{}` — fixed). Custom-call UX in the Apps UI is a UI-level check for launch day.
- [x] Testnet faucet plan documented (docs/faucet-plan.md — off-chain drip bot design, not implemented)
- [x] Docs complete: protocol spec, SECURITY.md, operator guide, economic parameters, node-ops, genesis ceremony, faucet plan, RPC audit, upgrade policy, CONTRIBUTING, CODE_OF_CONDUCT, CHANGELOG ✓

## P2 — Post-testnet / pre-mainnet backlog (tracked, not necessarily built)
- [ ] Public testnet metrics: finalized-head SLO, peer diversity, upgrade drill
- [ ] External audit engagement plan
- [ ] Governance path (referenda or council) before sudo removal on mainnet
- [ ] Light-client story (warp sync proof provider already wired; document)
- [ ] Explorer/indexer integration notes
- [ ] Deterministic builds (reproducible) and release signing

## Execution model
All work now lands directly on `devin/integration` in small verified commits (single-operator flow — no more child sessions). Integration merges into `main` via PR #7 once the merge policy is decided. Nothing lands without: (a) tests green, (b) a review pass over the delta, (c) docs updated in the same commit. Checkboxes get ticked only with a link to the commit/run that proves it.
