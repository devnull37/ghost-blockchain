# Testnet Readiness

This document is the launch checklist and smoke-test runbook for Ghost.

## Current Snapshot

State on `devin/integration`:

- Native pallet tests: available.
- Native node build: available.
- Embedded Wasm runtime build: available when the local Substrate C toolchain is installed.
- **Ghost-consensus E2E gate: `scripts/e2e-ghost.sh` passes on this branch** — it drives the live PoW path end to end (see the runbook section below for exactly what it proves).
- `scripts/e2e-local.sh`: legacy Aura-era gate, kept for history only. It no longer passes on this base: Aura is removed, an unflagged `--dev` node authors nothing (block production needs `--mine`), and the `ghost status`/`ghost mine` CLI subcommands it calls no longer exist (only `ghost verify-pow` remains).
- `ghost-consensus` engine crate (`consensus/ghost-consensus`): merged **and wired into `node/src/service.rs`** — `GhostPowAlgorithm` verifies every imported seal, `HeaviestChain` picks fork choice by `PowAux.total_difficulty`, `--mine` spawns the `sc-consensus-pow` mining worker plus grinding threads.
- `ghost-pow-primitives` crate (`primitives/ghost-pow`): merged — `GhostSeal`, `POW_ENGINE_ID`, `GhostPowApi`. The runtime implements `GhostPowApi::next_difficulty`; `e2e-ghost.sh` exercises it through `state_call`.
- `pallet-ghost-pqc`: merged **and composed into the runtime** (index 14) — ML-DSA-87 (FIPS-204) key registry with proof-of-possession, `pqc_attest` gated to bonded validators, no_std verifier compiled into the Wasm build. `RequirePqcKey = true`: `validate()` requires a registered key, and `select_validators` re-checks `has_pqc_key` at each selection so `revoke_pqc_key` cannot keep a seat. (Genesis stakers are seeded directly and unaffected.) The pallet's `PqcRequired` storage flag is policy metadata — the runtime's `ConstBool` is the authoritative gate.
- `pallet-ghost-consensus` (in runtime): the live staking/difficulty/reward pallet — bonded-stake validator selection via `pallet_session`, `Difficulty` retarget every 100 blocks, digest-decoded miner rewards (40% author / 60% validator split). All of these are exercised by `e2e-ghost.sh`.
- Live Ghost consensus path in the node service: running — PoW block production (`--mine`), PoW-verifying import queue, GRANDPA finality over the stake-selected session committee.
- Multi-node Ghost soak: `scripts/soak-ghost.sh` + `docs/soak-report.md` landed — a 4–5 node `--chain local` network with monitored invariants (finality advancing, bounded head spread, ≥1 peer, no finalized-hash forks) and scheduled miner kills. Its 300s QUICK profile passed twice; a 30–60 min full soak is still outstanding. `e2e-ghost.sh` covers the two-miner smoke scope only.

## Readiness Checklist

Mark each item green before calling the chain testnet-ready:

- [x] Native pallet tests pass.
- [x] Native node build works.
- [x] Embedded Wasm runtime builds with documented Substrate toolchain env.
- [x] `ghost-node --dev` boots cleanly with the embedded runtime.
- [x] Two local Aura/GRANDPA validators can peer, author, and finalize blocks. *(Historical — the Aura path has since been removed; `e2e-ghost.sh` is the live gate.)*
- [x] `ghost-consensus` engine crate + `ghost-pow-primitives` merged with unit/property tests.
- [x] `pallet-ghost-pqc` merged with unit tests (ML-DSA-87, PoP registration).
- [x] The node authors and finalizes through the Ghost consensus path (`ghost-consensus` wired into `service.rs`; Aura removed). Proven by `scripts/e2e-ghost.sh`.
- [x] `GhostPowApi::next_difficulty` implemented by the runtime; retarget driven by on-chain state. `e2e-ghost.sh` observes the block-200 adjustment via `state_call`.
- [x] Miner attribution is available for reward distribution (author decoded from the seal-bound pre-runtime digest). `e2e-ghost.sh` decodes per-block coinbases from `chain_getHeader`.
- [x] Reward distribution completes end to end — `e2e-ghost.sh` asserts free balances of the coinbase accounts (which are also genesis-staked validators) grow; the exact 40/60 split math is covered by pallet unit tests.
- [x] `pallet-ghost-pqc` is composed into the runtime and PQC verification works in the no_std/Wasm path. ML-DSA-87 verifies in the Wasm build; `validate()` and session selection both consult the registry.
- [x] Two or more Aura/GRANDPA nodes can form a network, author blocks, finalize, and survive a validator restart. *(Historical — superseded by the Ghost gate.)*
- [x] Two or more Ghost-consensus nodes can form a network, author blocks, finalize, and survive a restart — `e2e-ghost.sh` steps 3-6.
- [ ] The launch checklist is reproducible by someone who did not help build the branch. *(CI's `e2e`/`e2e-faults` jobs run the same scripts on a clean runner — that is the cold-run evidence; a human cold-run remains open.)*

## Minimum Launch Gate

Do not schedule a public testnet until all of the following are true:

1. The runtime builds without the `SKIP_WASM_BUILD` workaround.
2. The live node path uses Ghost consensus, not Aura/GRANDPA (the merged
   `ghost-consensus` crate drives the import queue; Aura is removed).
3. Reward accounting is correct on chain (digest-decoded author, 40/60 split).
4. `pallet-ghost-pqc` is in the runtime and ML-DSA-87 validation works in the
   Wasm build. *(Satisfied — `GhostPqc` at index 14, `RequirePqcKey`, and the
   bonded-validator `pqc_attest` gate all compile into the Wasm build.)*
5. `rtk scripts/e2e-ghost.sh` (the two-node Ghost-consensus gate) passes from a clean checkout.
6. `scripts/e2e-faults.sh` (adversarial gate) passes: miner halt/resume,
   validator offline/rejoin, and GRANDPA equivocation → slash + removal.
   *(Passed on spec-103 — A: frozen-best equilibrium held at fin=best−2
   under the default voting rules; B: finality pinned at 0 with 1/2
   committee, resumed on rejoin; C: equivocation report included,
   SlashRecords names Alice, bond 1e15→0 — see
   `docs/adversarial-report.md`.)*
7. `scripts/e2e-forged-seal.sh` passes: honest nodes deterministically
   reject blocks sealed with a corrupted `pre_hash` from a patched node.
   *(Passed on spec-103 — evil node self-authored 10 blocks, 16 honest
   headers scanned on each of two committee nodes with zero forged imports,
   finality reached 14 during the attack window.)*
8. `scripts/e2e-upgrade.sh` passes: a Wasm→Wasm `sudo set_code` upgrade
   lands on a live chain and finality continues on the new spec_version.
   *(Passed on spec-103 — full 1.73 MB runtime submitted as
   `sudo(system.set_code)` applied spec 103→104 in-block, best and
   finalized heads advanced on the new runtime, `GhostConsensus` storage
   version still 2. Operational hazard found and fixed in the drill: the
   installed @polkadot/types decodes a `Uint8Array`/`Buffer` `Bytes` arg
   as already-SCALE-encoded, so a wasm blob starting with `0x00` encodes
   as an EMPTY vec — `:code` stored zero-length wedges the chain
   (`UnexpectedEof`, nodes cannot even boot on that db). Always pass the
   wasm as a hex string — `api.tx.system.setCode(u8aToHex(code))` — and
   `e2e-upgrade.sh` now refuses to run against a node binary whose
   embedded spec doesn't match `runtime/src/lib.rs`.)*
9. `scripts/soak-ghost.sh` completes a ≥30-min multi-node soak with no
   invariant violations (finality advance, bounded head spread, ≥1 peer,
   no finalized forks) — see `docs/soak-report.md`.
10. Genesis machinery exists for a real allocation:
    `scripts/genesis-generate.sh` + ceremony checklist
    (`docs/genesis-ceremony.md`).

## Runbook

### 1. Prep the environment

Install the normal Rust and build prerequisites for Substrate work:

- `rustup`
- `clang`
- `curl`
- `git`
- `libssl-dev` or the platform equivalent
- `wasm32-unknown-unknown` target

The repository uses `rtk` for local commands when running commands manually.

The known-good Ubuntu package set includes:

```bash
sudo apt-get install -y clang
```

### 2. Run the Ghost-consensus E2E smoke gate

```bash
rtk scripts/e2e-ghost.sh
```

To reuse a prebuilt binary instead of compiling:

```bash
SKIP_BUILD=1 GHOST_NODE_BIN=$PWD/target/debug/ghost-node rtk scripts/e2e-ghost.sh
```

`MINING_THREADS` (default 4) sets grinding threads per miner.

What the gate proves on the live Ghost path:

1. `ghost-node` builds with the embedded Wasm runtime.
2. A `--dev --mine --miner-coinbase` node authors blocks whose headers carry both the `Seal(*b"pow_", GhostSeal)` digest and the `PreRuntime(*b"pow_", AccountId32)` miner digest — verified by decoding every header from `chain_getHeader` (the log hex contains `05706f775f` and `06706f775f`). A peered non-mining dev node imports the same blocks and authors nothing.
3. Two `--chain local` miners (Alice + Bob session committee) each author `pow_`-sealed blocks: the pre-runtime digests decode to both distinct coinbase accounts over time.
4. GRANDPA finality advances on both nodes with bounded lag behind the PoW best head, and each node reports exactly one peer.
5. The difficulty retarget executes on-chain: at the first real boundary (block 200; block 100 only stores the baseline) the script reads `LastRetargetTime` at the boundary block (the `now` the pallet captured in `on_initialize`) and at the block before it (the baseline), recomputes the pallet's expected difficulty from its own formula (elapsed-scaled, clamped ±4x, floored at `MinDifficulty`), and requires `GhostPowApi_next_difficulty` via `state_call`, the `GhostConsensus::Difficulty` storage item, and `RetargetsDone` to all agree with it. On slow debug blocks the correct outcome can be "floored at `MinDifficulty`" — the gate proves the retarget ran and computed correctly rather than only checking that the number moved.
6. Restart resilience: Bob is killed and rejoined on the same base path — it resyncs, keeps importing sealed blocks, resumes authoring, and finality advances.
7. Rewards: `System::Account` free balances of the miner coinbase accounts — which are also the genesis-staked session validators — increase across the window (miner reward + validator share minted per block).

CI's `e2e` job runs `scripts/e2e-ghost.sh` against the release `ghost-node` artifact (`SKIP_BUILD=1 GHOST_NODE_BIN=…`).

`scripts/e2e-local.sh` is the retired Aura-era gate. It stays in the repo for history but fails against the PoW node: its `ghost status`/`ghost mine` CLI calls no longer exist and a `--dev` node without `--mine` authors nothing. Use `e2e-ghost.sh` as the repeatable gate.

### 3. Run the native smoke build

```bash
rtk env BINDGEN_EXTRA_CLANG_ARGS=-I/usr/lib/gcc/x86_64-linux-gnu/13/include SKIP_WASM_BUILD=1 cargo build --bin ghost-node
rtk cargo test -p pallet-ghost-consensus
```

Expected result:

- The build succeeds.
- The pallet tests pass.

### 4. Try the full node build

```bash
rtk env \
  WASM_BUILD_WORKSPACE_HINT=$PWD \
  LIBCLANG_PATH=/lib/llvm-18/lib \
  BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/x86_64-linux-gnu/13/include -I/usr/include/x86_64-linux-gnu -I/usr/include" \
  cargo build --bin ghost-node
```

Expected result:

- The embedded runtime compiles.
- The binary is ready for a real `--dev` or multi-node launch.

If this fails because the Wasm toolchain or `clang` is missing, stop here and fix the build environment before moving on.

### 5. Boot a local dev node

```bash
./target/debug/ghost-node --dev --tmp --mine --miner-coinbase 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY
```

Expected result:

- The chain starts.
- RPC comes up.
- Blocks are PoW-authored (`--mine` is required; an unflagged dev node produces nothing) and finalized by GRANDPA over the genesis session committee.
- `ghost_getConsensusMode` reports `ghost-pow ... + GRANDPA` and a `next_difficulty` value.

If the node exits with `Development wasm not available`, the readiness gate is not met yet.

### 6. Multi-node smoke test

`scripts/e2e-ghost.sh` steps 3-6 already automate this for `--chain local`
(Alice + Bob mining validators, bootnode peering, restart). To run it by
hand, bring up two clean nodes with separate base paths, unique ports, and
the shared `local` chain spec:

- Node A: `ghost-node --chain local --alice --validator --mine --miner-coinbase <Alice SS58> --node-key <key>`
- Node B: `ghost-node --chain local --bob --validator --mine --miner-coinbase <Bob SS58> --bootnodes <A peer>`

Do not run two `--dev` nodes as peers — both would hold Alice's session
keys and GRANDPA logs a self-equivocation. Use `--chain local` for
multi-node runs.

Expected result:

- Peering succeeds (each node's `system_health.peers == 1`).
- Both miners author `pow_`-sealed blocks; `chain_getHeader` digests show
  `05706f775f`/`06706f775f` with both coinbase accounts as authors.
- Finality advances on both, and blocks continue to be produced after a
  restart.

### 7. Functional checks

The old `ghost status`/`ghost balance`/`ghost stake`/`ghost mine` CLI
subcommands were removed with the Aura path. What exists today:

- `ghost verify-pow <block-hash>` — re-verifies a block's PoW seal against
  the local chain database (recomputes the hash/difficulty check, prints
  the decoded miner and nonce).
- RPC `ghost_getNodeStatus`, `ghost_getConsensusMode`,
  `ghost_getPqcStatus` — honest engine/difficulty/PQC reporting.

Expected result:

- `ghost verify-pow` prints `pow seal: VALID` for any imported sealed block.
- The RPC methods report the live Ghost path (PoW authoring, heaviest-chain
  fork choice, current `next_difficulty`) rather than placeholder status.

## Exit Criteria

The branch is ready for a testnet when the following are all true:

- The readiness checklist is fully green.
- The runbook can be repeated without tribal knowledge.
- The docs describe the chain as testnet-ready only after the code path matches the claim.
