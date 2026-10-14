# Ghost Security Review — Round 1

Status: adversarial review of the Ghost v2 stack on `devin/integration`
(`consensus/ghost-consensus`, `primitives/ghost-pow`,
`pallets/pallet-ghost-consensus`, `pallets/pallet-ghost-pqc`, runtime v2, node
service). The PoW node-service wiring (`--mine`/`--miner-coinbase`, PoW import
queue, GRANDPA on `HeaviestChain`) is landing in a parallel PR — where a
finding depends on it, it is marked *pending service merge*.

Scope note: `git fetch origin pull/13/head` returned "no such ref" — the
service diff was not published at review time, so service findings cover the
code that exists on the base.

## Summary

| Severity | Count | Fixed here |
|---|---|---|
| Critical | 1 | 1 |
| High | 2 | 2 |
| Medium | 4 | 0 |
| Low | 12 | 1 |
| **Total** | **19** | **4 changes** |

Fixed in the accompanying PR: C-1 (migration wiring), H-1 (unbonding slash
gap), H-2 (authorship/liveness wiring), L-3 (endianness doc). Everything else
is reported with a concrete fix suggestion but left for a scoped follow-up
per the surgical-fix-only rule.

---

## Critical

### C-1 — v1→v2 storage migration exists but is never wired; upgrade panics on `Difficulty` decode

- **File:** `runtime/src/configs/mod.rs:76` (`type SingleBlockMigrations = ()`),
  migration lives at `pallets/pallet-ghost-consensus/src/migrations.rs`
  (`MigrateToV2`).
- **Bug:** The pallet stores `Difficulty` as `U256` at storage version 2, but
  the pallet's `#[pallet::storage]` prefix is unchanged from v1, where
  `Difficulty` was a `u64`. The `MigrateToV2` translation that kills the
  legacy keys and seeds `Difficulty = MinDifficulty` was written but never
  registered — `SingleBlockMigrations` was `()`. On the first block of the
  upgraded runtime, `Difficulty::<T>::get()` decodes an 8-byte SCALE `u64` as
  a 32-byte `U256`: `parity-scale-codec` errors on short input, and the
  pallet's `unwrap_or_default`-free accessors panic or, depending on the
  accessor, `on_finalize`/`next_difficulty` traps → **permanent chain halt**
  on upgrade (every block execution fails at the retarget path once
  `block % 100 == 0`, and the runtime API call `next_difficulty` — used by
  the client `difficulty()` path — errors immediately on every block).
- **Attack scenario:** not an attacker scenario — a self-inflicted brick at
  the first v1→v2 upgrade. The v1 fields it protects against (`Difficulty`,
  `CurrentPhase`, `SlashingRecords`, `BlockHeaders`, `ValidatorStakes`,
  `DoubleSignReports`, `ValidatorPqcPublicKeys`, …) are exactly the keys a
  pre-upgrade chain holds.
- **Fix applied:** `type SingleBlockMigrations =
  (pallet_ghost_consensus::migrations::MigrateToV2<Runtime>,)` in
  `runtime/src/configs/mod.rs`. The migration already stamps
  `StorageVersion::new(2)` and is idempotent for post-v2 runs (existing tests
  `migration_is_noop_when_already_v2`, `migration_clears_all_legacy_storage`
  cover both directions).

## High

### H-1 — `unbond` grants slash immunity: `on_offence` only cut `Bonded`, and fully-unbonded offenders skipped even the chill

- **File:** `pallets/pallet-ghost-consensus/src/lib.rs`, `on_offence`
  (previously ~lines 860–903).
- **Bug:** The loop read `bonded = Bonded::<T>::get(who)`, computed
  `amount = fraction * bonded`, then `if amount.is_zero() { continue; }` —
  skipping the burn **and** the chill/`SlashRecords` write. `Unbonding`
  chunks were never touched even though `unbond` only moves stake into a
  time-locked queue (`HoldReason::Staking` is still held until
  `withdraw_unbonded`).
- **Attack scenario:** A validator who commits an offence (GRANDPA
  equivocation, sustained im-online unresponsiveness — both flow here via
  `pallet_offences`) calls `unbond` for the full amount before the offence
  lands (offences are reported async within `ReportLongevity` = 400 blocks ≈
  20 sessions; unbonding the full amount takes effect immediately in
  `Bonded`). The offence then slashes `Bonded == 0` → `continue` → the
  offender loses **nothing** and is not even chilled, and every unbonding
  chunk pays out in full at maturity. If they were still an active validator
  they keep validating (they were never removed from `Candidates`/
  `ActiveValidators`). The unbonding queue length (`MaxUnbondingChunks` = 16,
  `UnbondingPeriod` = 14 days in block-count terms) is far longer than the
  report window, so a validator can even unbond *proactively* upon committing
  the offence and still be within the slashable window.
- **Fix applied:** `on_offence` now slashes `fraction * Bonded` **plus**
  `fraction *` each `Unbonding` chunk (per-chunk `mul_floor`, drained chunks
  removed via `retain`), burns the total held amount once, and the
  early-`continue` only fires when the offender is *genuinely* stakeless
  (no bonded, no unbonding — preserving the existing
  `on_offence_skips_accounts_with_no_bond` semantics). Chill + `SlashRecords`
  + `Slashed` event now run for any offender that had stake, including a
  bonded==0 offender whose chunks were cut to zero (they must still leave
  the validator sets; `amount` may legitimately be 0 when `fraction == 0`).
- **Regression tests:** `on_offence_slashes_unbonding_chunks_too`,
  `on_offence_full_unbond_does_not_escape_chill_or_slash`,
  `on_offence_drains_all_chunks_at_full_slash` in
  `pallets/pallet-ghost-consensus/src/tests.rs`.

### H-2 — `pallet_authorship::FindAuthor = ()`: authored blocks never credit validators, so im-online liveness is heartbeat-only

- **File:** `runtime/src/configs/mod.rs:178` (previously `type FindAuthor = ()`),
  fed by `pallets/pallet-ghost-consensus` `block_author` decode rules.
- **Bug:** `pallet_im_online`'s liveness check is
  `ReceivedHeartbeats::contains_key(v) OR AuthoredBlocks[v] != 0`
  (`frame/im-online/src/lib.rs::is_online_aux`). `AuthoredBlocks` is written
  by `pallet_authorship`'s `EventHandler::note_author`, which calls
  `T::FindAuthor::find_author` on the block's pre-runtime digests. With
  `FindAuthor = ()` (`FindAuthor for ()` returns `None`), **no authored block
  ever counts** — a validator is only "online" if a heartbeat extrinsic
  arrived in the session.
- **Attack scenario / operational consequence:** Any validator whose
  heartbeat transaction is dropped, front-ran out of the pool, or simply
  unlucky within `HeartbeatAfter`/session timing is reported unresponsive and
  — via `ReportUnresponsiveness = Offences` — is slashed and chilled by
  `on_offence`, *even while they are honestly mining blocks*. On a PoW chain
  this compounds: a large miner authoring most blocks but without a heartbeat
  pipeline gets slashed repeatedly. This is a liveness-slashing footgun that
  punishes honest validators; under adversarial pool manipulation it becomes
  an economic attack (grief rival validators into unresponsive slashes and
  soak their `Bonded`/`Unbonding`).
- **Fix applied:** new `pallet_ghost_consensus::PowFindAuthor<AccountId>`
  implementing `FindAuthor` by decoding the first decodable
  `PreRuntime(POW_ENGINE_ID, AccountId)` digest — identical semantics to the
  pallet's own `block_author` (which governs rewards), and wired via
  `type FindAuthor = pallet_ghost_consensus::PowFindAuthor<AccountId>`.
  Upstream `PowVerifier::check_header` already guarantees ≤1 pow_
  pre-runtime and a decodable payload before block execution, so runtime
  decode cannot diverge from import acceptance.
- **Regression tests:** `pow_find_author_decodes_pow_preruntime`,
  `pow_find_author_ignores_non_pow_and_undecodable`.

## Medium

### M-1 — Two divergent fork-choice implementations on equal-total ties

- **Files:** `consensus/ghost-consensus/src/select_chain.rs`
  (`HeaviestChain::best_header` orders leaves by `(total desc, number desc,
  hash asc)`) vs `sc-consensus-pow`'s implicit tie rule inside
  `PowBlockImport::import_block` (equal total difficulty →
  `algorithm.break_tie`, default `false` → keep existing best, i.e.
  *earliest-seen wins*).
- **Issue:** The crate ships `HeaviestChain` (deterministic hash tiebreak)
  while the import path decides best-ness on earliest-seen. If the pending
  service merge wires `ForkChoiceStrategy::Custom(HeaviestChain)` for the
  select chain while import still applies the earliest-seen rule — or two
  nodes pick different `ForkChoiceStrategy` — nodes **disagree on the best
  tip** on every equal-work fork until one side accrues more work. Equal-total
  ties are routine in PoW (same difficulty ⇒ same per-block work
  contribution), so this is not an edge case; it produces split mining effort
  and a short-lived but real consensus divergence.
- **Attack scenario:** An attacker watching a 2-block race publishes a
  conflicting tie block; a fraction of the network adopts each tip,
  amplifying a selfish-mining withholding strategy.
- **Fix suggestion:** Make `break_tie` semantics identical between the two
  implementations (implement `break_tie` on `GhostConsensusAlgorithm` to
  return `(number, hash)` ordering consistent with `HeaviestChain`), or drop
  `HeaviestChain`/`ForkChoiceStrategy::Longest` from the service path so a
  single ordering rules both sides. Verify at service merge that
  `ForkChoiceStrategy::Custom` is used, not `Longest` (which ignores total
  difficulty entirely).

### M-2 — `difficulty()` falls back to the parent's stored aux difficulty on runtime-API failure

- **File:** `consensus/ghost-consensus/src/algorithm.rs` `difficulty()` —
  runtime API `GhostPowApi::next_difficulty(parent)` error → aux lookup →
  `difficulty == 0` → `initial_difficulty`.
- **Issue:** When the runtime API call fails (wasm trap, API version
  mismatch, transient executor error), the verifier accepts seals computed
  against the **parent block's** difficulty — which after a retarget boundary
  may be up to 4× easier (or 4× harder, rejecting honest blocks) than the
  value `on_finalize` wrote. Combined with `aux.difficulty == 0 → initial`,
  a corrupt/missing aux entry silently re-opens difficulty at genesis level.
- **Attack scenario:** At a retarget boundary an attacker times a block whose
  seal meets the *old* 4×-easier difficulty while honest miners target the
  new one — accepting it requires the API failure to also affect honest
  verifiers, so this is a consistency/availability bug more than a forging
  primitive, but it widens disagreement windows.
- **Fix suggestion:** Treat runtime-API failure as a verification error
  (reject), not as "use parent's". If a fallback is wanted for sync
  robustness, at minimum distinguish "API says difficulty X" from "API
  unreachable" and log loudly; never silently accept stale work factor.

### M-3 — Retarget timestamp window lets miners compress ~2× faster or stretch ~6% per 100-block window

- **Files:** `pallets/pallet-ghost-consensus/src/lib.rs` `retarget()`
  (`elapsed = now.saturating_sub(LastRetargetTime)`), bounded upstream by
  `pallet_timestamp::check_inherent` (`t ≤ verifier_now + 30_000 ms`,
  `t ≥ prev + MinimumPeriod` = 2500 ms here).
- **Issue:** The miner picks `now` inside that window per block. Over one
  100-block retarget window (`expected = 500_000 ms`): minimum achievable
  `elapsed` = `99 × 2500` ≈ 247.5 s → `old × 500000/247500` ≈ **×2 increase**
  per window; maximum = `expected + ~30 s` lookahead ⇒ ≈ **×0.94 decrease**
  per window. Effects compound across consecutive windows. Direction matters:
  compressing raises difficulty for *everyone* (self-harm unless used to
  price out a specific rival hashrate range); stretching is capped by the
  30 s lookahead so is weak but free.
- **Attack scenario:** A cartel with >50% hash and slightly-faster clocks
  drives difficulty up ~2×/window to squeeze out minority miners, or each
  miner nudges +30 s ahead per block for a persistent ~6% discount.
- **Fix suggestion:** Also pin `LastRetargetTime` to the *median* of the
  window (or clamp `elapsed` to `[expected/4, 4*expected]` symmetrically —
  the clamp already exists for difficulty but the same clamp on elapsed
  removes the drift asymmetry), and consider `MinimumPeriod =
  SLOT_DURATION` (5000 ms) instead of `SLOT_DURATION/2` to halve the
  compression bound. At minimum, document the bound.

### M-4 — Empty validator set stalls GRANDPA; genesis stakers bypass session-key requirement

- **Files:** `pallets/pallet-ghost-consensus/src/lib.rs` `new_session`
  (returns `Some(vec![])` when no candidates qualify → `pallet_session`
  installs an empty authority set → GRANDPA can never reach the 2/3
  threshold → **no finality, forever**, while PoW block production
  continues); `runtime/src/genesis_config_presets.rs` + `GenesisConfig`'s
  `stakers`/`InitialValidators` path, which inserts validators without the
  `validate()` extrinsic's `SessionKeys`/`PqcProvider` checks.
- **Attack scenario:** All candidates `chill` (or are chilled by slashes) in
  the same session → next `new_session` returns the empty set → GRANDPA votes
  can never finalize; the chain keeps producing PoW blocks but loses
  finality — indistinguishable from a liveness DoS. A misconfigured genesis
  produces validators that cannot sign GRANDPA/heartbeats at all (keyless),
  which *also* stalls finality from block 0.
- **Fix suggestion:** In `new_session`, when `select_validators` returns an
  empty set, fall back to `InitialValidators`/`ActiveValidators` (same
  genesis-fallback logic already used at session 0) rather than
  `Some(vec![])` — an empty committee is never better than the previous one.
  In genesis build, run the same session-key presence check `validate()`
  applies (or at least warn loudly) instead of trusting `stakers` blindly.

## Low

- **L-1 — `block_author` skips undecodable `pow_` pre-runtimes while the
  client rejects headers carrying a second one**
  (`lib.rs` `find_map` vs upstream `find_pre_digest` →
  `MultiplePreRuntimeDigests`). Unreachable today because import rejects the
  header first; if the reward path is ever run standalone it diverges.
  Suggest `find` on id, then `decode_all`, mirroring upstream.
- **L-2 — `AccountId::decode` (trailing bytes allowed) in `block_author` and
  `PowFindAuthor`** tolerates a padded payload where upstream
  `verify`/`decode_all` would reject — same unreachable-divergence class as
  L-1. Cheap hardening: `decode_all` + reject, since a miner has no reason to
  pad its own digest.
- **L-3 — `GhostSeal` doc said "little-endian U256"** while the code uses
  `U256::from_big_endian` (`primitives/ghost-pow/src/lib.rs`). Docs-only, but
  this exact ambiguity is how PoW endianness bugs ship. **Fixed here.**
- **L-4 — Slashed validators can re-`validate` immediately.** `SlashRecords`
  is a ledger, not a gate; nothing checks it in `validate()`. A slashed
  attacker re-enters next session with fresh stake — mirroring
  pallet-staking's chilled-until-unslash convention is one option (slash
  period), or require `SlashRecords` absence within `ReportLongevity`.
- **L-5 — `node/src/miner.rs` is dead demo code** using `u64` difficulty and
  `min(cfg, header)` threshold semantics different from the real
  work-factor rule. If the service PR doesn't replace it wholesale, delete
  it — a maintainer wiring it up later inherits a wrong algorithm.
- **L-6 — `u128` saturation edge in `mul_div_floor` reward share.** The
  author/validator split uses exact `mul_div_floor` and mints exactly
  `BlockReward`, but validator-share sums to `share` over `total_bonded`
  where `bonded > u128::MAX` saturates the share term. Only reachable with
  absurd balances; note for the audit trail.
- **L-7 — Aux `total_difficulty`/difficulty default(0) on missing/corrupt
  data** (`aux.rs`): a node with a damaged aux DB treats itself as a fresh
  sync and can be fed a "best chain" that isn't — standard PoW aux-trust
  caveat, worth a `warn!` at minimum.
- **L-8 — PQC pallet is compiled but not in the runtime**
  (`PqcProvider = ()`, `RequirePqcKey = false`, pallet absent from
  `construct_runtime!`), so all its gates are dormant; and inside the
  pallet: `pqc_attest` has no validator gating (TODO'd), attestation has no
  expiry/domain separation beyond the raw message, and `register_pqc_key`
  takes no deposit (registry spam costs nothing). None of this is live —
  capture before the pallet is actually wired in.
- **L-9 — `fetch_seal`/`best_header` interaction on tie leaves** errors if
  the chosen best leaf lacks a trailing seal — unreachable because verified
  blocks always carry one, but the helper is easy to misuse elsewhere.
- **L-10 — `mint_into`/`mint_reserves`/`burn_held` errors are swallowed**
  (`let _ =`) throughout rewards/slashing. `Mutate` failures are logically
  impossible today (unconstrained balance ops), but a future `Currency`
  impl could fail; prefer `defensive` asserts or propagating.
- **L-11 — `withdraw_unbonded` uses `Precision::BestEffort`** on the hold
  release: if a competing freeze/hold blocked part of the release the
  remainder stays held with no recovery path except re-calling. Minor.
- **L-12 — `RetargetInterval = 0` division-by-zero is a config-time footgun**
  (`n % RetargetInterval::get()` panics → every `on_finalize` traps → halt).
  Const-configured so it can't be exploited, but a `const _: () =
  assert!(RetargetInterval > 0)` guard costs nothing.

## Audited and cleared (no bug)

- **Seal position/multiplicity:** upstream `check_header` pops the last
  digest item and requires `Seal(POW_ENGINE_ID)`; a second pow_
  `PreRuntime` is rejected with `MultiplePreRuntimeDigests`. Authorship is
  bound inside the hashed `pre_digest`, so stealing a seal requires
  re-grinding under a new account — PoW cost is unchanged.
- **Seal replay across blocks/forks:** the hash input includes `pre_hash`
  (bound to the header's parent) — a seal from block N cannot validate on
  another branch.
- **`PowAux::increment`** uses `saturating_add`; total difficulty cannot
  wrap.
- **Reward accounting:** `distribute_block_reward` mints exactly
  `BlockReward` — the 60% share remainder goes to the author; no dust mint.
  Rewards pay once per block via `on_finalize` only (no extrinsic path).
- **`block_author` vs seal verify:** both decode `AccountId32` from the same
  `PreRuntime(pow_, …)` — the rewarded account is the one that paid the PoW
  cost.
- **Unbonding timing** uses block numbers (`unlock_at`), so timestamp skew
  cannot release stake early.
- **Offence dedup:** `pallet_offences`' `ConcurrentReportsIndex` rejects
  replayed reports; per-offender `slash_fraction` is applied at most once
  per offence key. `slash_fraction` is `Perbill` so a single offence can
  never exceed 100%; multiple offences are bounded by
  `ConcurrentReportsIndex` + chunk draining being idempotent.
- **`MaxSetIdSessionEntries` (20) == `ReportLongevity / SESSION_PERIOD`
  (400/20):** equivocation reports can always be submitted within longevity.
- **`bond`/`bond_extra` bounds, `validate` gates, `select_validators`
  tie-break (stake desc, account asc)** — all verified consistent.
- **PoP correctness:** `GHOST-PQC-POP` domain + `who.encode()` is signed and
  verified under the *submitted* public key — no cross-account replay and no
  key-swap front-running (registry keys are per-account).
- **Pallet retarget vs client `compute_next_difficulty`:** the pallet's
  branch structure mirrors the client's `mul_div_floor` + ±4× clamp +
  `MinDifficulty` floor within all realistic ranges; they diverge only past
  U256 saturation (unreachable).
- **Timestamp floor/ceiling:** `check_inherent` rejects `t >
  verifier+30 s` and `t < prev+MinimumPeriod` — bounds verified against
  stable2407.
- **Empty-set GRANDPA safety:** verified absent — see M-4.

## Tests added in this PR

- `pallets/pallet-ghost-consensus`: `on_offence_slashes_unbonding_chunks_too`,
  `on_offence_full_unbond_does_not_escape_chill_or_slash`,
  `on_offence_drains_all_chunks_at_full_slash` (H-1);
  `pow_find_author_decodes_pow_preruntime`,
  `pow_find_author_ignores_non_pow_and_undecodable` (H-2);
  `retarget_exact_clamp_boundaries` (pins the ±4× boundary at exact
  equality — `elapsed*4 == expected` must increase, `elapsed == expected*4`
  must decrease).
- `consensus/ghost-consensus`: `mining_and_verify_agree_on_accept_and_reject`
  proptest — `hash_meets`(mining) and `verify_seal`(import) must reach the
  same verdict for every `(seal, difficulty)` pair, including `d = 0`.
