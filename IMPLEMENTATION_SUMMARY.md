# Ghost Blockchain — Implementation Summary

Current state of the tree on `devin/integration` (spec_version 103). This
doc describes what is *built and verified*; the launch-gate checklist with
what remains open lives in `docs/testnet-readiness.md`, and the per-change
history is `CHANGELOG.md`.

## Consensus engine — live

- **PoW block production** via `sc-consensus-pow`: `ghost-node --mine
  --miner-coinbase` runs the mining worker + grind threads; every imported
  block's `Seal(b"pow_", GhostSeal{nonce, pre_hash})` is verified by
  `GhostPowAlgorithm` — invalid seals are rejected at import (proven by
  `scripts/e2e-forged-seal.sh`).
- **Heaviest-chain fork choice** on cumulative difficulty with a
  deterministic `pre_hash` tie-break shared by both selection paths.
- **GRANDPA finality** over the PoW chain, voted by a `pallet_session`
  committee re-selected every 20 blocks from top bonded candidates
  (with empty-selection fallbacks — the chain can't brick its committee).
- **Aura is fully removed**; `e2e-local.sh` is retired.

## Runtime

- `pallet-ghost-consensus`: holds staking, deterministic selection,
  retarget every 100 blocks, digest-decoded 40/60 rewards,
  `OnOffenceHandler` slashing incl. unbonding chunks, bounded storage.
- `pallet-ghost-pqc` (index 14): ML-DSA-87 registry in the Wasm build;
  `validate()` requires a key; `pqc_attest` requires bonded stake.
- Governance stack (spec-104, indices 15–20): `pallet_utility`,
  `pallet_scheduler`, `pallet_preimage`, Council = `pallet_collective` I1
  + `pallet_membership` I1, `pallet_democracy`. Council is seeded at
  genesis from the validator committee; democracy is the sovereign path
  (public proposals → referenda → root enactment via Scheduler). Council:
  external proposals 1/2, default-pass 3/5, fast-track 2/3, cancel 2/3,
  single-member veto; instant path is unanimity-only.
- Session/offences/im-online wired; `PowFindAuthor` attributes real
  authors; equivocation reports auto-submit via offchain tx pool.

## Verification status

- Unit/property: 72 workspace tests + 4 fuzz targets (~17M execs).
- Live e2e: `e2e-ghost.sh` (7 checks), `e2e-faults.sh` (3 adversarial
  scenarios), `e2e-forged-seal.sh` (invalid-PoW rejection),
  `e2e-upgrade.sh` (spec 103→104 live set_code), `e2e-governance.sh`
  (full council+democracy+scheduler lifecycle), `soak-ghost.sh`
  (40-min profile; QUICK passed twice, full run pending).
- CI: fmt/clippy/test/wasm-build/build/e2e/e2e-faults/e2e-governance/
  audit/deny + weekly soak; release pipeline on `v*` tags.
- Security: round-1 (19 findings, criticals fixed) + round-2 reviews in
  `docs/security-review/`.

## Still open before a public testnet claim

- Full ≥30-min soak run and first pass of `e2e-faults.sh` +
  `e2e-forged-seal.sh` on the final tree.
- The `testnet` chainspec preset + genesis ceremony allocation
  (`docs/genesis-ceremony.md`).
- Runtime-upgrade drill (Wasm→Wasm `set_code`) and a no-op-migration test.
- ~~Weight regeneration via `benchmark pallet`~~ — done: `weights.rs` for both
  Ghost pallets is now CLI/Wasm-measured on this tree (chill benchmark
  corrected to match the session-boundary chill design; `pqc_attest` measured
  with the bonded-validator gate).
- See `docs/testnet-readiness.md` for the authoritative open list.
