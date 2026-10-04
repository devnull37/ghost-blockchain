# Ghost Production Plan

Release bar: the standard a top-tier organization would apply before shipping a public network. Not "it compiles and a demo passes" — provable consensus, provable economics, reproducible ops, and a security posture that survives adversarial review. Every item below is an acceptance-gated checkbox; nothing is claimed until demonstrated.

## P0 — Blocking for any testnet claim

### Consensus core (must be real, not simulated)
- [ ] PoW block production live in the node service (`sc-consensus-pow`), Aura fully removed (pallet, api, keys, inherents, service wiring, genesis presets)
- [ ] Seal verification on import at every node; invalid seals rejected deterministically across nodes (proven by e2e with a forged-seal block attempt)
- [ ] Heaviest-chain fork choice via total difficulty; tie-break rule documented and tested
- [ ] GRANDPA finality over the PoW chain; finalized head advances under mining contention (2+ miners)
- [ ] Stake-driven validator committee via `pallet_session`; committee rotates with stake changes; GRANDPA set follows sessions
- [ ] Difficulty retargeting deterministic on-chain; retarget math has unit tests + property tests (no overflow, clamp bounds hold)
- [ ] Miner attribution unspoofable: author decoded from seal-bound pre-runtime digest; reward cannot be claimed by an extrinsic
- [ ] Rewards: 40% miner / 60% active validators, minted on-chain; remainder handling documented; issuance accounting test asserts total minted per block
- [ ] Staking: bond/bond_extra/unbond/withdraw_unbonded with holds + unbonding period; validate/chill; MinStake enforced
- [ ] Slashing: GRANDPA equivocation → offence → stake slash + chill; downtime heartbeat → slash; all evidence verifiable, no extrinsic-reported slashing
- [ ] All consensus-relevant storage bounded; no unbounded iteration in any hot path (review-signed-off)
- [ ] Real weights: benchmarks for every extrinsic, generated `weights.rs` checked in, block weight budget respected

### Correctness & safety engineering
- [ ] `cargo test --workspace` green; `cargo fmt --check`; `cargo clippy -- -D warnings` zero warnings
- [ ] Property tests (proptest): difficulty retarget bounds/monotonicity, reward split conservation (minted == distributed + remainder), seal verify accept/reject, stake-weighted selection distribution
- [ ] Fuzz target for seal decode + digest parsing (bounded input, no panic on arbitrary bytes)
- [ ] e2e (`scripts/e2e-local.sh`) on the Ghost path: 2 miners + 2 validators peer, author PoW blocks (seal digests verified via RPC), finalize, rewards land in `system_account`, validator restart+rejoin+finalize
- [ ] Soak (`scripts/soak.sh`): ≥4 nodes, ≥30 min, mixed restarts, finalized-head agreement, no finality stall
- [ ] Failure-mode tests: miner halts (finality continues/liveness behavior documented), validator offline (downtime slash fires), equivocation injected (slash fires)

### Ops baseline
- [ ] CI on every PR: fmt, clippy, tests, node build, e2e-smoke
- [ ] Dockerfile builds `ghost-node` and runs a dev node; compose file for local multi-node
- [ ] `ghost-node --version`, `--help`, `build-spec`, `export-chain-spec` all work; `chain-info` sane
- [ ] docs/testnet-readiness.md reflects the true gate with reproduction steps a stranger can run

## P1 — Required before public testnet launch

### Security & economics hardening
- [ ] Written threat model (docs/threat-model.md): 51% analysis, selfish-mining exposure of the fork-choice rule, long-range/finality assumptions, stake concentration risks, PQC scope (what is and isn't quantum-hardened)
- [ ] Economic parameter doc: issuance schedule, fee model, slash parameters, retarget constants — each with rationale
- [ ] `cargo deny`/`cargo audit` in CI (advisories, licenses, bans); `unsafe` remains forbidden
- [ ] External-style security review of consensus code paths (independent reviewer agents + checklist in docs/security-review.md), findings tracked to closure
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
Orchestrator assigns one workstream per child session; each child branches off `main`, opens one PR per item, keeps `cargo test --workspace` + e2e green. Nothing merges without: (a) tests green, (b) review pass (a reviewer child + orchestrator check), (c) docs updated in the same PR. Checkboxes get ticked only with a link to the merged PR/commit that proves it.
