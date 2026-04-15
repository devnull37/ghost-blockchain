# Ghost Blockchain

Ghost is a Substrate-based blockchain under active development. The current branch contains a Ghost consensus pallet, CLI helpers, and native build/test support, but it is not yet a live hybrid PoW/PoS testnet.

## Current State

- The node service still uses Aura for block authoring and GRANDPA for finality.
- The `pallet-ghost-consensus` code path contains Ghost PoW/PoS/PQC logic, but it is not yet the block production engine.
- Native pallet/runtime tests and a full embedded-Wasm node build are available with the documented Substrate toolchain.
- A local Aura/GRANDPA two-validator smoke test passes via `scripts/e2e-local.sh`.
- Dilithium5 verification is present for native/std paths, but the runtime no_std/Wasm PQC path is not yet a completed testnet-ready feature.

## What Works Now

- `pallet-ghost-consensus` unit tests.
- Embedded-Wasm node builds.
- Single-node dev authoring.
- Two-node local authoring and GRANDPA finality on the Aura/GRANDPA path.
- Ghost CLI helpers such as `ghost status`, `ghost mine`, `ghost stake`, and `ghost balance` in native mode.
- Development account setup for Alice and Bob in the chain spec.

## What Is Still Pending

- Wiring Ghost consensus into live block authoring and finality.
- Reliable reward attribution and distribution through the live block path.
- Multi-node E2E testing on the actual Ghost consensus path.
- Runtime PQC verification that works in no_std/Wasm.

## Build

Use the local smoke test first:

```bash
rtk scripts/e2e-local.sh
```

For a manual full build:

```bash
rtk env \
  WASM_BUILD_WORKSPACE_HINT=$PWD \
  LIBCLANG_PATH=/lib/llvm-18/lib \
  BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/x86_64-linux-gnu/13/include -I/usr/include/x86_64-linux-gnu -I/usr/include" \
  cargo build --bin ghost-node
```

Run the pallet tests:

```bash
rtk cargo test -p pallet-ghost-consensus
```

For a release build:

```bash
rtk cargo build --release --bin ghost-node
```

## Run

The native binary can be used for local inspection:

```bash
./target/debug/ghost-node ghost status --detailed
./target/debug/ghost-node ghost mine --threads 1 --difficulty 18446744073709551615
```

Run the repeatable local E2E smoke test with:

```bash
rtk scripts/e2e-local.sh
```

## Testnet Readiness

See [docs/testnet-readiness.md](docs/testnet-readiness.md) for the checklist and runbook that gate testnet launch.

## Architecture Snapshot

- `node/`: client, CLI, RPC, chain spec, and service wiring.
- `runtime/`: FRAME runtime composition.
- `pallets/pallet-ghost-consensus/`: Ghost consensus logic, reward rules, slashing, and staking primitives.

## Background

Ghost targets 5-second blocks, a 10 GHOST block reward, a 40/60 miner-to-staker split, and a minimum stake of 1 GHOST. Those are the intended economics and protocol goals; the docs in this repository now call out where the current implementation has reached the live path and where it still has work left.
