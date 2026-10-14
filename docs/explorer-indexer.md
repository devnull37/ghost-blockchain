# Explorer / indexer integration notes

Ghost is a stock-metadata Substrate chain: any FRAME-compatible indexer
(Subsquid, subscan, polkadot.js apps) works against the standard RPC. The
Ghost-specific surface an explorer should handle:

## Metadata & display

- `system_properties` in every chainspec ships `ss58Format: 42`,
  `tokenDecimals: 12`, `tokenSymbol: GHOST` — render balances in GHOST, not
  planck.
- Runtime metadata is standard (`state_getMetadata`); all custom pallets
  (`ghostConsensus` idx 8, `ghostPqc` idx 14, governance stack 15–20)
  appear with normal events/extrinsics/storage.

## PoW attribution — the Ghost-specific part

- **Block author is in the digest, not the extrinsics.** Each header's
  `digest.logs` carries `PreRuntime(b"pow_", SCALE(AccountId32))` — decode
  it for the miner. There is no `author` extrinsic; indexers reading only
  extrinsics will show blocks as authorless.
- **Seal**: `Seal(b"pow_", GhostSeal{nonce, pre_hash})` — the pre_hash is
  the candidate the miner ground; useful for explorer "work" display.
- **Difficulty** lives at `ghostConsensus.difficulty()` (U256 work factor,
  larger = harder; retargets every 100 blocks).

## Rewards

- No reward *events*: miner (40%) and validator-committee (60%) coinbase
  splits are credited in-block through `pallet_balances` deposits —
  indexers should diff `system.account` free balances across the block or
  read the `balances.Deposit` events that fire for each credit.

## Governance

- Standard `democracy`/`council`/`scheduler`/`preimage` pallets —
  subsquid's `DemocracyProposal`/`CouncilMotion` handlers work unchanged.
- Council membership = `councilMembership.members` (seeded from genesis
  validators).

## Indexing footguns (verified empirically on this chain)

- Session boundary = 20 blocks (`SESSION_PERIOD`); committee changes are
  `session.NewSession` events, not extrinsics.
- Equivocation slashes arrive via `offences`/im-online offences pipeline —
  watch `ghostConsensus.Slashed`-adjacent balance decrements, not a
  dedicated slash call.
- RPC rate guidance: archive indexing should run against a dedicated
  `--pruning archive` node; public RPC endpoints should not serve bulk
  `state_getKeys` ranges (see `docs/node-ops.md` firewall posture).
