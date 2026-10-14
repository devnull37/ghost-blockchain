# Public-testnet metrics & SLOs

Service-level objectives for the public testnet, measured from the stock
Substrate Prometheus endpoint (`--prometheus-port 9615`, `/metrics`).
Alert wiring maps to the runbook table in `docs/node-ops.md`.

## SLOs

| Objective | Target | Metric | Why this number |
|---|---|---|---|
| Block production | ≥ 99% of 5s slots filled in any 30-min window | `substrate_block_height{status="best"}` delta vs wall clock | PoW difficulty 1e6 target ≈ one solution per slot on reference HW; >1% misses = miner fleet problem |
| Finality lag | `best - finalized` ≤ 12 blocks p95 over 24h | `substrate_block_height{status="finalized"}` vs `status="best"` | 40-min soak measured avg 2.3 / max 12 on a 5-node LAN — hold public network to the observed ceiling |
| Finality stalls | zero stalls > 5 min per epoch | finalized delta = 0 for > 5 min | >5 min stall = committee liveness failure, page on-call |
| Peer connectivity | validator ≥ 3 peers sustained | `substrate_sub_libp2p_peers_count` | isolated validator = equivocation/partition risk |
| Peer diversity | no single /24 > 40% of p2p connections | libp2p peer addr census (off-node script) | eclipse resistance at network layer |
| Upgrade cadence | runtime-upgrade drill passes monthly | `e2e-upgrade.sh` on current release | drills rot if not run against the live spec |

## Finalized-head SLO (the chain's headline number)

Public promise: **finalized head keeps within 12 blocks of best head,
p95, per UTC day** — mirroring the soak evidence in
`docs/soak-report.md`. Breach = committee offline or consensus bug;
treat as sev-2, runbook `docs/node-ops.md` "finalized stalls" row.

## Measurement notes

- Scrape interval 15s; keep 30d retention minimum to observe retarget
  behavior across difficulty epochs (retarget every 100 blocks).
- `ghost_getNodeStatus` / `ghost_getConsensusMode` RPCs give the same
  numbers for smoke checks where Prometheus isn't deployed
  (`scripts/node-metrics.sh`).
- Difficulty is NOT an SLO — it self-adjusts; monitor
  `ghostConsensus.difficulty()` only for unexpected jumps (e.g. > 4x per
  epoch = hashrate surge, expected on miner onboarding).

## Upgrade drill cadence

Monthly: `scripts/e2e-upgrade.sh` against the current release tag —
proves the Wasm→Wasm set_code path still works against the shipping
spec before any real upgrade referendum relies on it.
