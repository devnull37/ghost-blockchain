# Node Operations

Running `ghost-node` in production: sizing, storage, metrics, upgrades,
incidents. Role-specific runbooks (session keys, PQC, slashing) are in
`docs/operator-guide.md`.

## Hardware sizing

| Role | CPU | RAM | Disk | Notes |
|---|---|---|---|---|
| Full node (RPC/archive) | 4 vCPU | 8 GB | NVMe, archive growth unbounded | pruning off for explorers |
| Validator | 4 vCPU dedicated | 8 GB | 100 GB+ NVMe | no oversubscribed cores — GRANDPA vote latency matters |
| Miner | mining threads + 2 for sync | 8 GB | 100 GB+ | `--mining-threads` = grind workers; size to all spare cores |

## Storage / pruning

- `--pruning archive` (default for `--chain local` is full state) — keep
  for RPC/indexer nodes.
- `--pruning 256` (or higher) — bounded recent-state retention for
  validators; finalized history is still in the block DB, only old *state*
  is dropped. PoW chains need deep block history for `verify-pow` and
  reorg depth — do not prune block bodies.
- Backups = a stopped node's `db/` dir under `--base-path`. Never restore a
  backup onto a node holding live GRANDPA keys while the original is still
  running — duplicate authority = equivocation = slash.

## Metrics (Prometheus)

`--prometheus-port 9615` (default) exposes `/metrics`. Alert minimums:

| Metric | What breaks without it |
|---|---|
| `substrate_block_height{status="best"}` advancing | PoW stall — no miners winning |
| `substrate_block_height{status="finalized"}` advancing | committee problem (offline/keys) |
| `substrate_sub_libp2p_peers_count` ≥ 1 | partitioned node |
| `substrate_sub_libp2p_is_major_syncing` = 0 | sync wedged |

The decisive pair is `best` vs `finalized` divergence: `best` advancing
with `finalized` pinned = committee liveness failure (see
`docs/economic-parameters.md` "Liveness failure semantics");
both frozen = mining failure (all miners stopped) — different incidents,
different pages.

## Telemetry

`--no-telemetry` for private runs. For a public testnet the telemetry sink
is `--telemetry-url` — configure the org's endpoint in the systemd unit,
not per-invocation.

## Upgrades (binary)

1. Stop the node (SIGTERM — clean DB close).
2. Swap the binary (keep `--base-path` and `--node-key` identical — peer
   id and DB are tied to them).
3. Start with identical flags; the node resumes from best.
4. Validator keys live in `base-path/chains/<chain>/keystore` — they must
   survive upgrades; wiping the base path = new authority = you must
   `set_keys` again or you'll stop voting (and eventually get an
   im-online offence).

Runtime (Wasm) upgrades are a separate procedure — see
`docs/runtime-upgrade-policy.md`.

## Incident runbook

| Symptom | Likely cause | First action |
|---|---|---|
| `finalized` stalls, `best` advances | committee member(s) offline | check own GRANDPA voter logs; verify peers on other validators |
| `best` stalls | all miners stopped / difficulty spike | restart miners; check `ghost_getConsensusMode` `next_difficulty` vs observed hashrate |
| Node won't peer | bad `--node-key`/`--bootnodes` | confirm bootnode peer id matches |
| Equivocation offence logged | duplicate authority keys on two live nodes | kill one immediately, assess the slash |
| Node OOMs | too many `--mining-threads` | drop to cores-1 |

## Backup / restore

- What matters under `base-path`: `db/` (chain data — regenerable),
  `chains/<chain>/keystore` (authority keys — NOT regenerable).
- Back up the keystore offline; the db re-syncs.
- Restoring a keystore to a second live node while the first runs is an
  equivocation — always kill the original first.
