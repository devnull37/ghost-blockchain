# Economic Parameters

Every value below is the on-chain truth, not a target: each row names the
runtime constant (or pallet item) that defines it. Change them in
`runtime/src/configs/mod.rs` / `runtime/src/lib.rs` and bump `spec_version`.

## Units

| Name | Value | Defined at |
|---|---|---|
| `UNIT` (1 GHOST) | 1_000_000_000_000 planck (10^12) | `runtime/src/lib.rs` |
| `MILLI_UNIT` | 1_000_000_000 | `runtime/src/lib.rs` |
| `MICRO_UNIT` | 1_000_000 | `runtime/src/lib.rs` |
| `EXISTENTIAL_DEPOSIT` | `MILLI_UNIT` (0.001 GHOST) | `runtime/src/lib.rs`, `pallet_balances` config |

## Block production and time

| Name | Value | Defined at |
|---|---|---|
| `SLOT_DURATION` / target block time | 5_000 ms | `runtime/src/lib.rs` (`MILLI_SECS_PER_BLOCK`), `TargetBlockTimeMs` |
| `MinimumPeriod` (pallet_timestamp) | 1 ms | `runtime/src/configs/mod.rs` |
| Max timestamp future drift | ~30 s (substrate `MAX_TIMESTAMP_DRIFT`) | `pallet_timestamp` |

`MinimumPeriod = 1` is deliberate: PoW has no slot to protect, and the
default `SLOT_DURATION/2` ratchet pinned `Now` at the +30 s drift cap,
rejecting ~30–40% of honest winning seals as `TooFarInFuture`
(`docs/soak-report.md`). Timestamps remain monotonic (`now > prev`) and
future-drift-capped; the difficulty retarget's ±4x clamp bounds any
residual miner influence on `elapsed`.

## Proof-of-work difficulty

| Name | Value | Defined at |
|---|---|---|
| Work factor | larger = harder; validity is `hash <= U256::MAX / difficulty` | `ghost-consensus` `GhostPowAlgorithm` |
| `MinDifficulty` (genesis value + floor) | 1_000_000 | `runtime/src/configs/mod.rs` |
| `RetargetInterval` | 100 blocks | `runtime/src/configs/mod.rs` |
| Formula | `new = old * clamp(expected_ms / elapsed_ms, 1/4, 4)`, floored at `MinDifficulty`, saturating at `U256::MAX` | `ghost_pow_primitives::compute_next_difficulty` (shared with the node, fuzz-verified bit-identical) |
| `elapsed` source | `pallet_timestamp::Now` at the retarget block vs `LastRetargetTime` | `pallet-ghost-consensus::retarget` |

The first boundary (block 100) only establishes the baseline timestamp;
the first real retarget fires at block 200.

## Rewards

| Name | Value | Defined at |
|---|---|---|
| `BlockReward` | `10 * UNIT` per block | `runtime/src/configs/mod.rs` |
| Author share | 40% + rounding remainder | `pallet-ghost-consensus::distribute_block_reward` |
| Validator share | 60%, pro-rata by bonded stake over `ActiveValidators` | same |
| Empty-committee fallback | validator share minted to the pallet account | same |
| Author attribution | decoded from `PreRuntime(b"pow_", AccountId32)` digest — unspoofable by extrinsics | `block_author`, `PowFindAuthor` |

Rewards are minted (inflationary issuance, no fee recycling yet).

## Staking / validator set

| Name | Value | Defined at |
|---|---|---|
| `MinStake` | `UNIT` (1 GHOST) | `runtime/src/configs/mod.rs` |
| `MaxStake` | `1_000_000 * UNIT` | `runtime/src/configs/mod.rs` |
| `MaxValidators` (committee size) | 100 | `runtime/src/configs/mod.rs` |
| `MaxValidatorCandidates` | 1_024 | `runtime/src/configs/mod.rs` |
| `MaxUnbondingChunks` | 16 | `runtime/src/configs/mod.rs` |
| `UnbondingPeriod` | `14 * DAYS` = 14 * 24 * 720 = 241_920 blocks (~14 days at 5 s) | `runtime/src/configs/mod.rs` |
| Selection | top `MaxValidators` candidates by bonded stake, tie-broken by account id ascending — fully deterministic | `select_validators` |
| `SESSION_PERIOD` | 20 blocks (~100 s at target) | `runtime/src/configs/mod.rs` |
| `ReportLongevity` (equivocation window) | `20 * SESSION_PERIOD` = 400 blocks | `runtime/src/configs/mod.rs` |
| PQC requirement | `RequirePqcKey = true`: `validate()` requires a registered ML-DSA-87 key; `select_validators` re-checks per selection | `runtime/src/configs/mod.rs` |
| `pqc_attest` | bonded validators only (`bonded >= MinStake`) | `BondedValidatorAdapter` |

Bonded stake is a `pallet_balances` *hold* (funds stay spendable-visible
but locked), not a reserve. `unbond` queues a chunk matures after
`UnbondingPeriod`; `withdraw_unbonded` releases it.

## Slashing

| Name | Value | Defined at |
|---|---|---|
| Handler | `pallet_offences::Config::OnOffenceHandler = GhostConsensus` | `runtime/src/configs/mod.rs` |
| Offences covered | GRANDPA equivocation (offchain auto-report), im-online unresponsiveness | runtime wiring |
| Slash fraction | supplied by the reporting offence (per-offender `Perbill`) | offence report |
| Scope | bonded stake AND all unbonding chunks (they stay held, so they stay slashable — `unbond` grants no immunity) | `on_offence` |
| Records | `SlashRecords` bounded vec (`MAX_SLASH_RECORDS`, oldest dropped) | pallet |
| Post-slash | offender chilled out of `Candidates`; `ActiveValidators` (the seated committee) is *not* mutated mid-session — removal takes effect at the next `new_session` selection | `remove_candidate`, `new_session` |

## Liveness failure semantics

- **All miners stop**: PoW best head freezes; GRANDPA finalizes up to it
  and stops — never past it (proven by `e2e-faults.sh` scenario A).
- **Committee member(s) offline**: with N=2 both votes are required;
  finality pins while best keeps advancing, and resumes when the member
  rejoins (scenario B).
- **Empty selection**: `new_session` falls back to the seated committee,
  then to `InitialValidators` — the chain never bricks its GRANDPA set
  (M-4).
