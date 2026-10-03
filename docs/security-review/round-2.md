# Security Review — Round 2

Scope: every change landed on `devin/integration` after `round-1.md` was
written — the M-4 session-selection fix, `MinimumPeriod = 1`, the fork-choice
pre-hash tie-break, the shared `compute_next_difficulty`, `pallet-ghost-pqc`
runtime wiring (registry + `pqc_attest` bonded gate + selection-time
recheck), and the adversarial e2e harness. Method: line-level review of each
delta plus a re-read of every storage/state transition they touch.

## Findings fixed in this pass

**R2-1 — `BondedValidatorAdapter` hardcoded `UNIT` instead of the configured
`MinStake`.** The pqc `pqc_attest` gate and the consensus `validate()` gate
would silently diverge if `MinStake` ever moved. Fixed: the adapter reads
`<Runtime as pallet_ghost_consensus::Config>::MinStake::get()`.

**R2-2 — `e2e-faults.sh` scenario C asserted "Alice chilled" via
`Candidates`, which was vacuous.** Genesis validators join via `stakers`
config, not `validate()`, so Alice was never in `Candidates`; and by the M-4
deferred-removal design, chilling does not mutate `ActiveValidators`
mid-session either. The assertion now verifies Alice's account id inside the
`SlashRecords` blob — i.e. the offence was attributed to her specifically.

## Verified (no issue found)

- **`new_session` fallbacks (M-4).** Empty selection → `ActiveValidators`
  → `InitialValidators`. `PendingValidators` is always written before the
  equality check, and `Some(set)`/`None` ordering means a gratuitous
  authority-set change can never be emitted. `start_session` mirrors
  `Pending` into `Active`, so the two can never desync.
- **`on_offence` slash coverage.** Slashes `fraction` of `Bonded` *and* of
  every `Unbonding` chunk (chunks stay held, so they stay slashable —
  `unbond` grants no immunity). `burn_held` bookkeeping balances exactly:
  held = bonded + Σchunks before and after. `SlashRecords` is bounded.
- **Reward split.** 40% author (digest-decoded, unspoofable) / 60% pro-rata
  over `ActiveValidators` by bonded stake; U256 intermediates; remainder to
  the author; empty-committee share to the pallet account. No dust path.
- **Retarget math.** Node and pallet share
  `ghost_pow_primitives::compute_next_difficulty` — the fuzz-discovered
  `prev * expected` overflow divergence is gone by construction.
- **PQC gating.** `validate()` requires a key under `RequirePqcKey`;
  `select_validators` re-checks `has_pqc_key` every selection, closing the
  `revoke_pqc_key`-mid-session loophole. `pqc_attest` requires bonded
  stake ≥ `MinStake`.
- **`PowFindAuthor`.** im-online sees the real PoW author, not `()` — the
  heartbeat/authorship starvation bug stays fixed.

## Residuals — accepted, with reasoning

**R2-R1 — Miner influence on `Now` → retarget `elapsed`.** `MinimumPeriod=1`
leaves monotonicity + the 30 s future-drift cap as the only timestamp
constraints; a miner could skew `elapsed` within a retarget window. Impact
is clamped to ±4x per 100 blocks by `compute_next_difficulty` itself, and
the alternative (`MinimumPeriod = SLOT_DURATION/2`) provably rejected
~30–40% of honest seals. Accepted; documented in
`docs/economic-parameters.md`.

**R2-R2 — Mid-session deseat window.** A seated validator who revokes their
PQC key, unbonds below `MinStake`, or is chilled keeps voting until the next
`new_session` boundary (20 blocks). This is the designed deferred-removal
semantics — identical to `pallet_staking`'s era-boundary model — and the
alternative (mutating the live GRANDPA set mid-session) is exactly the
desync M-4 fixed. They cannot escape slashing in the window.

**R2-R3 — `RequirePqcKey` + zero registered keys freezes the committee.** If
every candidate lacks a registered key, `select_validators` returns empty
and the M-4 fallback keeps the seated committee — the chain cannot brick
its GRANDPA set, but it also cannot rotate. Accepted: correct failure mode
(freeze, not halt), and genesis stakers are intentionally exempt from the
key requirement since they are seeded directly.

**R2-R4 — Genesis committee trust.** `InitialValidators`/`stakers` bypass
`validate()`'s checks (no session keys lookup, no PQC requirement). Fine
for a genesis-declared committee; a real network spec must seed bonded
stakers deliberately rather than relying on dev accounts — tracked under
the chainspec/publish work item.

## Pointers for round 3

- `SlashRecords` oldest-drop policy vs `ReportLongevity = 400`: records are
  evidence, not enforcement — confirm UI/RPC consumers don't treat dropped
  records as "no offence".
- `block_author()` decodes the *first* `pow_` PreRuntime digest. A block
  carrying two is a producer-side bug the pallet would silently tolerate —
  worth a consensus-side assert if custom producers ever ship.
