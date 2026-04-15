# Testnet Readiness

This document is the launch checklist and smoke-test runbook for Ghost.

## Current Snapshot

- Native pallet tests: available.
- Native node build: available.
- Embedded Wasm runtime build: available when the local Substrate C toolchain is installed.
- Aura/GRANDPA local E2E smoke test: passing via `scripts/e2e-local.sh`.
- Live Ghost consensus path in the node service: still pending.
- Full Ghost-consensus multi-node E2E testnet: not ready yet.

## Readiness Checklist

Mark each item green before calling the chain testnet-ready:

- [x] Native pallet tests pass.
- [x] Native node build works.
- [x] Embedded Wasm runtime builds with documented Substrate toolchain env.
- [x] `ghost-node --dev` boots cleanly with the embedded runtime.
- [x] Two local Aura/GRANDPA validators can peer, author, and finalize blocks.
- [ ] The node authors and finalizes through the Ghost consensus path.
- [ ] Miner attribution is available for reward distribution.
- [ ] Reward distribution completes end to end.
- [ ] Dilithium5 verification works in the runtime no_std/Wasm path.
- [x] Two or more Aura/GRANDPA nodes can form a network, author blocks, finalize, and survive a validator restart.
- [ ] Two or more Ghost-consensus nodes can form a network, author blocks, finalize, and survive a restart.
- [ ] The launch checklist is reproducible by someone who did not help build the branch.

## Minimum Launch Gate

Do not schedule a public testnet until all of the following are true:

1. The runtime builds without the `SKIP_WASM_BUILD` workaround.
2. The live node path uses Ghost consensus, not Aura/GRANDPA.
3. Reward accounting is correct on chain.
4. PQC validation works in the runtime build.
5. A two-node smoke test passes from a clean checkout.

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

### 2. Run the repeatable local E2E smoke test

```bash
rtk scripts/e2e-local.sh
```

Expected result:

- The node builds with the embedded Wasm runtime.
- Ghost CLI smoke checks pass.
- A `--dev` node authors blocks.
- Alice and Bob local validators peer, author blocks, and finalize blocks.

This validates the current Aura/GRANDPA-backed chain path. It does not validate live Ghost PoW/PoS consensus.

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
./target/debug/ghost-node --dev --tmp
```

Expected result:

- The chain starts.
- RPC comes up.
- Blocks are authored and finalized by the intended consensus path.

If the node exits with `Development wasm not available`, the readiness gate is not met yet.

### 6. Multi-node smoke test

Bring up two clean nodes with separate base paths, unique ports, and a shared chain spec.

Suggested pattern:

- Node A: Alice or the first validator authority.
- Node B: Bob or the second validator authority.
- Connect Node B to Node A using the peer ID printed in Node A's startup logs.

Expected result:

- Peering succeeds.
- Both nodes stay in sync.
- Blocks continue to be produced after a restart.

### 7. Functional checks

Run through the user-facing checks:

- `ghost status --detailed`
- `ghost balance`
- `ghost stake`
- `ghost mine`

Expected result:

- The commands report live chain state rather than placeholder status.
- The outputs match the actual consensus path on the chain.

## Exit Criteria

The branch is ready for a testnet when the following are all true:

- The readiness checklist is fully green.
- The runbook can be repeated without tribal knowledge.
- The docs describe the chain as testnet-ready only after the code path matches the claim.
