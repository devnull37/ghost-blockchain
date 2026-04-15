# AGENTS.md

This repository uses `rtk` as the command wrapper for local shell work. Prefer:

```bash
rtk cargo test -p pallet-ghost-consensus
rtk scripts/e2e-local.sh
rtk env \
  WASM_BUILD_WORKSPACE_HINT=$PWD \
  LIBCLANG_PATH=/lib/llvm-18/lib \
  BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/x86_64-linux-gnu/13/include -I/usr/include/x86_64-linux-gnu -I/usr/include" \
  cargo build --bin ghost-node
```

Current build reality:
- Full embedded-Wasm node builds pass with the documented Substrate C toolchain environment.
- `scripts/e2e-local.sh` is the repeatable local smoke gate. It builds the node, checks Ghost CLI, boots a dev node, boots Alice/Bob local validators, verifies peering, authoring, GRANDPA finality, and verifies Bob can restart/rejoin/finalize.
- `clang`/LLVM must be installed locally. On this Ubuntu environment, `LIBCLANG_PATH=/lib/llvm-18/lib` and the include paths in the example above are known-good.
- `SKIP_WASM_BUILD=1` is still useful for quick native checks, but do not use it to claim node/E2E readiness.

Consensus reality:
- `node/src/service.rs` still authors blocks with Aura and finalizes with GRANDPA.
- `pallets/pallet-ghost-consensus` contains Ghost PoW/PoS/PQC pallet logic, but it is not yet the node's live block production engine.
- Do not describe the chain as production hybrid PoW/PoS until the service imports, validates, and authors blocks through the Ghost consensus path.
- The Aura/GRANDPA local chain can peer, author, finalize, and survive a validator restart.
- Ghost pallet miner tracking, staking escrow, reward-path tests, and PQC-size fixes exist, but they are still pallet/runtime logic rather than the node's consensus engine.
- Dilithium5 verification is present for native/std builds. Runtime Wasm/no_std PQC verification is intentionally disabled with `PqcRequired = false` until a deterministic no_std verifier is implemented and benchmarked.

Next todos:
- Design the real Ghost consensus engine before coding broad changes. Document block authoring, import verification, fork choice, PoW seal format, PoS validator-selection timing, finality interaction, equivocation handling, and reward attribution.
- Replace or extend the Aura authoring path only after the Ghost import queue can reject invalid PoW/PoS blocks deterministically across nodes.
- Move miner attribution from pallet-only bookkeeping into the live block authoring/import path so rewards cannot be spoofed by extrinsics.
- Add on-chain reward accounting tests that cover miner rewards, staker rewards, slashing, failed validation, and replay/double-submit attempts.
- Implement Dilithium5 runtime verification with a no_std/Wasm-safe verifier. Add proof-of-possession to `register_pqc_key`, benchmark weights, and test valid/invalid signatures through runtime APIs.
- Add restart and persistence coverage for staking state, block-producer history, reward distribution, and slashing records.
- Keep `scripts/e2e-local.sh` green after every consensus or runtime change; extend it with Ghost-consensus-specific checks once the live engine exists.
- Before any public testnet claim, run a multi-node long soak with clean base paths, validator restarts, RPC checks, and finalized-head agreement.

Docs ownership:
- Keep `README.md`, `IMPLEMENTATION_SUMMARY.md`, and `docs/*` aligned with the current branch state.
- Put launch-readiness notes in `docs/testnet-readiness.md` and keep them tied to the actual code path, not the aspirational design.
- When writing readiness language, distinguish Aura/GRANDPA local smoke readiness from live Ghost PoW/PoS consensus readiness.
