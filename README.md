# Ghost Blockchain

Ghost is a Substrate-based PoW + PoS-committee chain: miners produce blocks
via real proof-of-work (`sc-consensus-pow`, work-factor difficulty), and a
stake-selected committee finalizes them via GRANDPA. Validator seats require
bonded stake **and** a registered ML-DSA-87 (FIPS-204) key.

## Consensus

- **PoW authoring**: `--mine` + `--miner-coinbase` — no stake or keys
  required. Seal = `DigestItem::Seal(b"pow_", GhostSeal{nonce, pre_hash})`;
  the miner's `AccountId32` rides in a seal-bound `PreRuntime` digest, so
  rewards can't be claimed by extrinsic.
- **Fork choice**: heaviest chain by cumulative difficulty, deterministic
  tie-break on embedded `pre_hash`.
- **Finality**: GRANDPA over the PoW chain, voted by a `pallet_session`
  committee selected each session from top bonded candidates.
- **Difficulty**: on-chain retarget every 100 blocks,
  `new = old * clamp(expected/elapsed, 1/4, 4)`, floored at `MinDifficulty`
  — computed identically in node and pallet via `ghost_pow_primitives`.
- **Rewards**: 10 GHOST/block — 40% to the digest-decoded author, 60%
  pro-rata over the seated committee.
- **Slashing**: GRANDPA equivocation (auto-reported by offchain workers)
  and im-online unresponsiveness slash bond + unbonding chunks, then chill.
- **PQC**: `pallet-ghost-pqc` registry in the runtime (index 14);
  `validate()` and `pqc_attest` both gate on it.

## Build

```bash
# Toolchain is pinned by rust-toolchain.toml (rustc 1.88.0 + wasm32).
# Needs clang/libclang + system deps — see docs/rust-setup.md.
cargo build --release --bin ghost-node
```

## Run

```bash
# Single dev miner (Alice committee, disposable state):
ghost-node --dev --tmp --mine --miner-coinbase 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY

# Two-node local committee + miners:
ghost-node --chain local --alice --validator --mine --miner-coinbase <ALICE SS58>
ghost-node --chain local --bob --validator --mine --miner-coinbase <BOB SS58> --bootnodes <A>
```

See `docs/operator-guide.md` (validators + miners) and
`docs/node-ops.md` (sizing, metrics, incidents).

## Verification gates

| Gate | What it proves |
|---|---|
| `cargo test --workspace` | pallet + consensus unit/property tests |
| `rtk scripts/e2e-ghost.sh` | 2-miner live PoW: seals, finality, retarget, rewards, restart |
| `rtk scripts/e2e-faults.sh` | miner halt / committee offline / equivocation→slash |
| `rtk scripts/e2e-forged-seal.sh` | honest nodes reject invalid PoW from a forger |
| `rtk scripts/soak-ghost.sh` | multi-node 40-min soak, restarts, fork agreement |
| CI | fmt, clippy, test, wasm-build, build, e2e, e2e-faults, audit, deny; weekly soak |

## Docs

`docs/` contains the protocol spec, threat model, design doc,
economic parameters, operator guide, node-ops, genesis ceremony, faucet
plan, RPC audit, upgrade policy, readiness checklist, soak + adversarial
reports, and both security reviews. `CHANGELOG.md` tracks spec_version.

## Status

Working toward public testnet: the gate list lives in
`docs/testnet-readiness.md` (what is proven vs what is still open).
Current `spec_version`: 102.
