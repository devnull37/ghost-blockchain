# AGENTS.md

This repository uses `rtk` as the command wrapper for local shell work. Prefer:

```bash
rtk cargo test -p pallet-ghost-consensus
rtk scripts/e2e-ghost.sh
rtk env \
  WASM_BUILD_WORKSPACE_HINT=$PWD \
  LIBCLANG_PATH=/usr/lib/llvm-14/lib \
  BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/x86_64-linux-gnu/13/include -I/usr/include/x86_64-linux-gnu -I/usr/include" \
  cargo build --bin ghost-node
```

Current build reality:
- Full embedded-Wasm node builds pass with the toolchain pinned in
  `rust-toolchain.toml` (rustc 1.88.0 + wasm32 target) plus clang/libclang.
- On this Ubuntu environment `LIBCLANG_PATH=/usr/lib/llvm-14/lib` and the
  include paths in the example above are known-good; `rtk` exports them.
- `SKIP_WASM_BUILD=1` is still useful for quick native checks, but do not
  use it to claim node/E2E readiness.

Consensus reality:
- `node/src/service.rs` produces blocks through `sc-consensus-pow`
  (`--mine` + `--miner-coinbase`) and finalizes them with GRANDPA voted by
  a `pallet_session` committee selected from `pallet-ghost-consensus`
  stake. Aura is fully removed — do not reference it as the live path.
- `pallets/pallet-ghost-consensus` is the chain's real economic layer:
  bonded stake, committee selection, difficulty retarget, digest-decoded
  40/60 rewards, slashing. `pallets/pallet-ghost-pqc` (index 14) gates
  `validate()` on a registered ML-DSA-87 key.
- Miner attribution comes from the `PreRuntime(b"pow_", AccountId32)`
  digest — never from extrinsics.
- `scripts/e2e-local.sh` is retired; the local gates are `e2e-ghost.sh`
  (two-miner smoke), `e2e-faults.sh` (adversarial), `e2e-forged-seal.sh`
  (invalid-PoW rejection), and `soak-ghost.sh` (multi-node soak).

Next todos (see `docs/production-plan.md` + `docs/testnet-readiness.md`):
- Full ≥30-min soak and first `e2e-faults.sh`/`e2e-forged-seal.sh` passes
  on the final tree.
- `testnet` chainspec preset with a real genesis allocation
  (`docs/genesis-ceremony.md`).
- Runtime-upgrade drill (Wasm→Wasm `set_code`) and a tested no-op
  migration (`docs/runtime-upgrade-policy.md`).
- Regenerate benchmarked weights via `benchmark pallet` whenever
  dispatchable logic changes after a benchmark run.
- Keep every gate green after each consensus or runtime change.

Docs ownership:
- Keep `README.md`, `IMPLEMENTATION_SUMMARY.md`, this file, and `docs/*`
  aligned with the current branch state.
- Put launch-readiness notes in `docs/testnet-readiness.md` and keep them
  tied to the actual code path, not the aspirational design.
- Never claim production or testnet readiness without the corresponding
  gate having run and passed on that tree.
