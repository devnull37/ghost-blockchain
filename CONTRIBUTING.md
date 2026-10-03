# Contributing

## Workflow

1. Branch off `main`. Keep diffs focused — one concern per PR.
2. Before pushing: `rtk cargo fmt --all`, `rtk cargo clippy --workspace
   --all-targets -- -D warnings`, `rtk cargo test --workspace`.
3. If the change touches the node binary's behavior, run
   `rtk scripts/e2e-ghost.sh`; if it touches failure modes, also
   `rtk scripts/e2e-faults.sh`.
4. Open a PR with a description written for a reader who has not seen the
   diff: what changed, why, what proves it works. Screenshots/logs for
   consensus behavior changes.
5. CI must be green: fmt, clippy, test, wasm-build, node build, e2e,
   e2e-faults, audit, deny.

## Rules that are non-negotiable

- **Never skip the Wasm build to claim readiness.** `SKIP_WASM_BUILD` is
  for quick native iterations only — `docs/testnet-readiness.md` is the
  honesty reference.
- **Bump `spec_version`** on any runtime-logic or metadata change
  (`docs/runtime-upgrade-policy.md`).
- **`unsafe` is forbidden** in this workspace.
- **New consensus-critical math goes in `ghost_pow_primitives`**, not
  side-by-side node/pallet copies — the fuzz suite proved how fast those
  drift.
- **Update docs in the same commit** that changes the behavior.
- Storage migrations ship with the layout change, gated by
  `StorageVersion`, with tests.

## Style

Follow the surrounding code. The codebase prefers: explicit error types
over panics, bounded storage everywhere, weights that come from real
benchmarks not guesses, and comments only where a decision would be
surprising without them (see `remove_candidate`'s deferred-removal
comment for the tone).

## Security reports

Do not open public issues for vulnerabilities — see `SECURITY.md`.
