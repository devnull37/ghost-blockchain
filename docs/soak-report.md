# Soak report — `scripts/soak-ghost.sh`

Long-run local-network soak for the PoW chain (`--chain local`), the
pre-testnet gate from `docs/production-plan.md` (P1 ops maturity).

## What the harness does

`scripts/soak-ghost.sh [duration_secs]` boots a 4–5 node network with
fresh base paths under `soak-data/`:

| node | flags | role |
| --- | --- | --- |
| `alice` | `--alice --validator --mine --miner-coinbase <Alice SS58>` | bootnode + GRANDPA committee + miner |
| `bob` | `--bob --validator --mine --miner-coinbase <Bob SS58>` | GRANDPA committee + miner |
| `miner1`, `miner2`* | `--mine --miner-coinbase <Charlie/Dave SS58>` | keyless miners (PoW authoring is open; they hold no session keys) |
| `rpcnode` | (no `--mine`, no `--validator`) | observer / RPC target |

\* `miner2` only in the default profile. Alice's fixed node-key is the
single bootnode; every miner uses a distinct `--miner-coinbase`.

The GRANDPA committee under `--chain local` is exactly {Alice, Bob} —
N=2 needs **both** voters for finality, so alice and bob are never
restarted. The adversity schedule only kills keyless miners.

### Monitored invariants (every 10s via JSON-RPC)

- finalized head strictly advances — stall limit is adaptive: 15s when
  recent block production is near-deterministic (<4s avg gap), else
  9× the recent average gap capped at 90s, plus a 60s grace window
  after each restart kill
- best-head spread across nodes ≤ 5 (excluding a node in restart grace)
- every node keeps ≥ 1 peer
- no two nodes report different `chain_getBlockHash` at the same
  finalized height (incremental watermark check)
- no node process dies outside the kill schedule; no node's RPC is dead
  for >3 consecutive ticks

### Adversity

At ~25% and ~60% of the run a keyless miner is `SIGKILL`ed for 30s and
restarted on the same base path. Post-run the script asserts the
restarted miner submitted new winning seals after its last restart.

### Outputs

`soak-data/report.csv` (per-tick per-node metrics),
`soak-data/summary.md` (this report's inputs), `soak-data/logs/*.log`
(full node logs), `soak-data/diagnostic-dump.txt` on failure.

### Usage

```bash
rtk scripts/soak-ghost.sh            # default profile: 2400s, 2 keyless miners (5 nodes)
QUICK=1 rtk scripts/soak-ghost.sh 300   # CI profile: ~5 min, 1 keyless miner (4 nodes)
GHOST_NODE_BIN=/path/to/ghost-node rtk scripts/soak-ghost.sh 900
```

If `target/release/ghost-node` is missing the script builds it
(`cargo build --release`) — the first run takes a while. `GHOST_NODE_BIN`
skips the build. `duration_secs` overrides the duration; a meaningful
full run is 30–60 min.

## Quick-run result (300s, QUICK=1)

PASSED — verified on this branch (two consecutive clean runs).

| metric | value |
| --- | --- |
| blocks produced | 125 |
| max finalized height | 123 |
| finality lag (best − finalized) | avg 2.8 / p95 6 / max 6 |
| max observed finalized-head stall | 12s (at the 10s tick granularity) |
| invariant violations | 0 |
| crashes / RPC hangs / hash forks | 0 |
| keyless-miner restarts | 2× miner1 (30s down each) — resynced and resumed authoring (9 new seals post-restart) |

Authoring was well distributed across the network (canonical `pow_`
pre-runtime digest attribution, heights 1..123): alice 35, bob 58,
miner1 34, rpcnode 0.

## Chain bugs / observations found

### `TooFarInFuture` — a large share of winning seals are wasted (worth fixing)

~30–40% of all winning seals are **rejected at import** by the
timestamp inherent check:

```
Unable to import mined block: Import failed: Checking inherents failed:
The timestamp of the block is too far in the future.
```

Observed counts in the 300s run: alice 33, bob 31, miner1 26 — vs
~127 successfully imported blocks.

Mechanism: `pallet_timestamp` requires `now ≥ parent_now +
MinimumPeriod` (`MinimumPeriod = SLOT_DURATION/2 = 2500ms`). Every
imported block therefore ratchets the chain's `Now` up by at least
2.5s regardless of the real elapsed time. Whenever actual block
production runs faster than ~2.5s/block (difficulty has dipped toward
`MinDifficulty` after retargets), `Now` outruns wall time until it
pins against the verifier's `MAX_TIMESTAMP_DRIFT = 30s` cap — at which
point every newly mined proposal is born "in the future" and rejected.
The mining thread reuses the same pinned timestamp, so rejects come in
bursts until wall time catches up.

Effect on the network: none fatal — finality kept advancing (max lag
6, stall 12s) and the effective block rate clamps near 2.5s. But it
wastes roughly a third of mining work and distorts the retarget
input: `pallet_ghost_consensus::retarget()` measures elapsed via the
chain's `Now`, which is systematically inflated during these bursts, so
retargets understate how fast blocks are really being found.

Suggested direction (not fixed here): derive the proposal timestamp
from `max(wall_clock, parent_now + MinimumPeriod)` clamped at
`verifier_wall + MAX_DRIFT` rather than letting the inherent push past
the drift cap, or bound how far `Now` can lead wall time in the
inherent provider.

**FIXED post-merge (commit `41abc2d`):** `MinimumPeriod` is now 1ms —
PoW has no slot to protect, so `Now` tracks miners' wall-clock
timestamps instead of ratcheting to the drift cap. Validated with the
same 300s quick soak on `devin/integration`: TooFarInFuture rejects
**90 → 0** (alice 33→0, bob 31→0, miner1 26→0); blocks produced
**125 → 410** in the same window (~0.7s/block effective — the seal
waste had been suppressing the rate ~3.3×); seals ≈ canonical authored
(151/149, 141/140, 120/120, near-zero wasted work); finality lag avg
3.4; both miner1 kill/restart cycles recovered. The remaining ~24
"Unable to import" lines are non-timestamp PoW races, not this bug.

### Adaptive stall limits are required for a PoW chain

The spec's fixed `3 × target block time` stall limit (15s) would
false-fail a healthy chain: at a 5s mean block interval ~5% of
Poisson gaps exceed 15s. The implemented limit (15s when the recent
average gap is <4s, else 9× recent average capped at 90s) had zero
false positives across the runs while still catching real stalls.

## Full-run expectations

- `QUICK=1` (CI): ~5 min, 4 nodes, kills at ~75s and ~180s elapsed.
- Default (2400s): 5 nodes, kills at ~10 and ~24 min. Expect ~400–600
  finalized blocks, finality lag avg 2–4 blocks, occasional
  `TooFarInFuture` bursts until the timestamp issue above is fixed.
- A PASS requires every invariant to hold for the whole duration; any
  violation aborts with a diagnostic dump into
  `soak-data/diagnostic-dump.txt` and exit 1.

## Full-run result (2400s, 5 nodes, release binary, spec-103)

PASSED — verified on `devin/integration`.

| metric | value |
| --- | --- |
| blocks produced (max best) | 797 |
| max finalized height | 795 |
| finality lag (best − finalized) | avg 2.3 / p95 3 / max 12 |
| max observed finalized-head stall | 43s (inside the adaptive limit) |
| invariant violations | 0 |
| crashes / RPC hangs / finalized-hash disagreements | 0 |
| keyless-miner restarts | miner1 + miner2 (SIGKILL, 30s down each) — both resynced AND resumed authoring (81 and 60 new canonical blocks post-restart) |

Canonical authoring spread (heights 1..795, `pow_` digest attribution):
alice 193, bob 216, miner1 197, miner2 193, rpcnode 0 (observer).
`TooFarInFuture` rejects: **0 across all miners** — the `MinimumPeriod=1ms`
fix holds at soak scale (~0.75s effective block rate sustained for 40 min).
The 43 `Unable to import` lines are lost fork-choice races (self-authored
on a parent that lost), not timestamp or seal failures; that is the
expected orphan rate at this production speed on localhost.

Harness fixes proven by this run (all landed in this run's script):
per-tick node sampling is now concurrent (sequential sampling baked
2–4s of production into the spread measurement), the sampler `wait` is
scoped to sampler pids (a bare `wait` dead-locked on the node processes),
a 30s boot head-spread grace covers post-warmup convergence, and node
memory is capped via `GHOST_NODE_MEM_ARGS` defaults
(`--db-cache 32 --max-runtime-instances 2 --runtime-cache-size 2` —
the 5-node soak peaks at ~1.4GB on an 8GB box).
