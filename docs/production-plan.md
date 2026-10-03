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
- [ ] All consensus-relevant storage bounded; no unbounded iteration in any hot path (review-signed-off) — round-1 review cleared the pallet; sign-off at round-2
- [x] Real weights: benchmarks for all 10 extrinsics + checked-in `weights.rs` — PR #15 (regenerate via node CLI pending)

### Correctness & safety engineering
- [x] `cargo test --workspace` green; `cargo fmt --check`; `cargo clippy -- -D warnings` zero warnings — CI green on PR #8; kept green through PR #13/#15
- [x] Property tests (proptest): retarget bounds/monotonicity, seal verify accept/reject vs mining — PR #9/#10 (reward-conservation + selection-distribution properties in pallet tests)
- [ ] Fuzz target for seal decode + digest parsing (bounded input, no panic on arbitrary bytes)
- [ ] e2e on the Ghost path (`scripts/e2e-ghost.sh`): 2 miners + committee peer, author PoW blocks (seal digests via RPC), finalize, rewards land, restart+rejoin — IN FLIGHT
- [ ] Soak (`scripts/soak-ghost.sh`): ≥4 nodes, ≥30 min, mixed restarts, finalized-head agreement, no finality stall — IN FLIGHT
- [ ] Failure-mode tests: miner halts (finality continues/liveness documented), validator offline (downtime slash fires), equivocation injected (slash fires) — partial (im-online path exists); adversarial e2e pending

### Ops baseline
- [x] CI on every PR: fmt, clippy, tests, node build, e2e-smoke — PR #8 (6 jobs green)
- [x] Dockerfile builds `ghost-node` + compose multi-node — PR #8 (docker build + compose verified)
- [x] `ghost-node --version`, `--help`, `build-spec`, `export-chain-spec` work — verified during service merge
- [ ] docs/testnet-readiness.md reflects the true gate — PR #12 updated sections; final pass after e2e/soak land

## P1 — Required before public testnet launch

### Security & economics hardening
- [x] Written threat model (docs/threat-model.md): adversary classes, per-attack mitigations vs residual risk, known-gaps register — PR #12
- [ ] Economic parameter doc: issuance schedule, fee model, slash parameters, retarget constants — each with rationale
- [ ] `cargo deny`/`cargo audit` in CI (advisories, licenses, bans); `unsafe` remains forbidden
- [ ] External-style security review: round-1 done — PR #14 (19 findings, C-1/H-1/H-2 fixed in place, docs/security-review/round-1.md); round-2 after e2e/soak + remaining mediums
- [ ] Session key ops documented: `set_keys`, key rotation, validator migration runbook
- [ ] RPC surface audit: unsafe methods denied by default, `--rpc-methods` guidance, no key material in RPC

### Runtime quality
- [ ] `spec_version`/`impl_version`/`transaction_version` bump policy documented; first runtime-upgrade (Wasm→Wasm) exercised on a devnet before testnet
- [ ] Storage migration framework convention (versioned storage, `OnRuntimeUpgrade` hooks) + a tested no-op migration
- [ ] Genesis ceremony doc for public testnet: allocation table, validator onboarding, bootnode list, chain spec JSON published in-repo (`chainspecs/`)
- [ ] SS58 prefix decision documented (registered prefix or justified default)
- [ ] Remove `pallet-template` and `sudo` from production preset (sudo kept only in dev/testnet spec with a documented removal plan)

### Ops maturity
- [ ] Telemetry endpoints configured + Prometheus metrics list documented (block height, finality lag, peers, mining hashrate)
- [ ] Node ops docs: hardware spec, `--pruning` guidance, archive vs full node, backup/restore, upgrade procedure, incident runbook
- [ ] Release pipeline: tagged release → reproducible binary + versioned Docker image + checksums + release notes template
- [ ] Polkadot.js Apps compatibility verified (metadata + custom types, ss58, signing)
- [ ] Testnet faucet plan documented (crate-independent, e.g. bot or pallet-gated drip) — not necessarily implemented
- [ ] Docs complete: whitepaper-grade protocol spec (`docs/protocol-spec.md`), validator guide, miner guide, builder/dev docs, security policy (`SECURITY.md`), contribution + code of conduct, changelog policy

## P2 — Post-testnet / pre-mainnet backlog (tracked, not necessarily built)
- [ ] Public testnet metrics: finalized-head SLO, peer diversity, upgrade drill
- [ ] External audit engagement plan
- [ ] Governance path (referenda or council) before sudo removal on mainnet
- [ ] Light-client story (warp sync proof provider already wired; document)
- [ ] Explorer/indexer integration notes
- [ ] Deterministic builds (reproducible) and release signing

## Execution model
Orchestrator assigns one workstream per child session; children branch off `devin/integration` (staging branch containing the ratified design docs + seeded deps), open ONE PR back to `devin/integration`, keep `cargo test` + e2e green. Integration merges into `main` via PR #7 once the foundation PR lands. Nothing merges without: (a) tests green, (b) review pass (a reviewer child + orchestrator check), (c) docs updated in the same PR. Checkboxes get ticked only with a link to the merged PR/commit that proves it.
