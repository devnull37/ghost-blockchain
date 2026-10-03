# Ghost Consensus Design

Status: ratified target architecture. All consensus work must implement this document; deviations require updating this file first.

## 1. Architecture

Ghost is a hybrid PoW/PoS chain on Substrate (polkadot-sdk `stable2407`):

- **Block production: real Proof-of-Work** via `sc-consensus-pow`. Miners grind a nonce against the block pre-hash; the import queue of every node verifies the seal before import. Fork choice is heaviest-chain by cumulative difficulty (`PowAux` aux data, `total_difficulty` comparison inside `PowBlockImport`), not longest-chain.
- **Finality: GRANDPA.** A PoS committee (the top stakers, selected by `pallet_session`) votes to finalize blocks. PoW produces candidates; stakers provide deterministic finality. This is the honest meaning of "PoS validation" on Ghost: finality votes, not per-block extrinsics.
- **PQC layer:** Dilithium5 / ML-DSA-87 key registry with on-chain proof-of-possession and validator attestation extrinsics, verified in no_std/Wasm. It strengthens validator identity; it does not replace GRANDPA.

Aura is removed entirely. Aura pallet, `AuraApi`, aura session keys, and aura service wiring are deleted. Timestamp `MinimumPeriod` becomes a constant (`2500ms`).

## 2. PoW seal format

The PoW `ConsensusEngineId` is `POW_ENGINE_ID` (`b"pow_"`) from `sp_consensus_pow`.

- **Pre-runtime digest**: `DigestItem::PreRuntime(POW_ENGINE_ID, author)` where `author` is the SCALE-encoded `AccountId32` of the miner. `start_mining_worker`'s `pre_runtime` parameter pushes this automatically. It is part of the header before hashing, so it is covered by the seal.
- **Seal digest**: `DigestItem::Seal(POW_ENGINE_ID, seal)` where `seal` is the SCALE encoding of `GhostSeal { nonce: u64 }`. Found via `MiningHandle::submit`.

`GhostSeal` is defined in `sp-ghost-consensus` (new primitives crate) so both client and runtime can decode it.

## 3. PoW algorithm (`GhostPowAlgorithm`)

Crate: `node/` sibling crate `consensus/` implementing `PowAlgorithm<Block>`.

- `Difficulty = sp_core::U256` (already implements `TotalDifficulty` via saturating add).
- **Difficulty is a work factor, not a target** (kulupu convention): larger = harder. `PowBlockImport` sums per-block difficulty into `total_difficulty` and picks the heaviest chain, so difficulty MUST be monotone in work — a target-style value (where easier = larger) would invert fork choice.
- `verify(parent, pre_hash, pre_digest, seal, difficulty)`:
  - `input = pre_hash.as_ref() ++ pre_digest ++ seal.encode()`
  - `hash = blake2_256(blake2_256(input))` (double-hash; deterministic, Wasm-safe)
  - `value = U256::from_big_endian(hash)`; valid iff `value.saturating_mul(difficulty) <= U256::MAX` (equivalent to `value <= MAX / difficulty`; overflow saturates to MAX and fails). `difficulty == 0` always fails.
  - `pre_digest` is the raw `PreRuntime(POW_ENGINE_ID, bytes)` payload = `SCALE(AccountId32)` miner id — decode it and bind it into the verification (a block with a malformed author digest is rejected client-side; rewards decode the same digest).
  - `preliminary_verify` returns `Ok(None)` (needs parent aux context — keep full verify).
  - `break_tie`: keep default `false` (earliest-seen wins).
- `difficulty(parent)`: calls the runtime API `GhostPowApi::next_difficulty(parent_hash)`; called twice per import — implement a small memoization; on failure fall back to `PowAux` parent's difficulty.

## 4. Difficulty adjustment (runtime-side, deterministic)

All nodes compute difficulty identically from on-chain state — `sc-consensus-pow` calls `difficulty(parent)` during import on every node, so it must be a pure function of the parent state.

- `pallet-ghost-consensus` storage: `Difficulty: U256` (**work factor** — SCALE encodes U256 as 32-byte LE), `LastRetargetTime: Moment`, `RetargetsDone: u32`.
- `RETARGET_INTERVAL = 100` blocks, `TARGET_BLOCK_TIME_MS = 5000`, clamp factor to `[1/4, 4]` per retarget.
- In `on_initialize`, when `block_number % RETARGET_INTERVAL == 0` and `RetargetsDone > 0` (skip genesis window):
  `new_difficulty = old_difficulty * clamp(expected_ms / elapsed_ms, 0.25, 4)` — blocks arriving too fast (elapsed < expected) INCREASE the work factor; too slow decreases it. Floored at `MIN_DIFFICULTY` (genesis constant, ≥ 1), saturated at U256::MAX. Use `expected_ms = RETARGET_INTERVAL * TARGET_BLOCK_TIME_MS` between the timestamps of the first and last block of the interval, guarded against `elapsed_ms == 0` (treat as max-up retarget).
- Genesis difficulty chosen so a dev machine mines in ~1-5s: `INITIAL_DIFFICULTY` constant in runtime config, overridable via chain spec `ghostConsensus.difficulty` genesis field.
- Runtime API `GhostPowApi`: `next_difficulty() -> U256` (SCALE U256 = 32-byte LE on the wire). `author_of` is NOT an api — attribution is pure digest decoding (see §5).

## 5. Miner attribution & rewards (unspoofable)

- In `on_finalize` (or `on_initialize` before rewards phase), the pallet reads `frame_system::Pallet::<T>::digest()`, finds `PreRuntime(POW_ENGINE_ID, bytes)`, decodes `AccountId32` → that is the **block author/miner**. No extrinsic can set it: it is bound into the seal hash input (§3 includes `pre_digest` in the hash input), so forging it requires re-winning the PoW.
- Block reward `BlockReward = 10 * UNIT` minted per block:
  - **40%** to the decoded author.
  - **60%** split among the **current validator set** (bounded — `pallet_session::validators()`, stake-weighted pro-rata). If the set is empty, the share goes to the pallet account as a reserve.
  - Rounding remainder goes to the author (prevents dust accumulation; document it).
- Remove the `submit_block`/`validate_block`/`report_misbehavior` simulation extrinsics and `ConsensusPhase`/`BlockHeaders`/`BlockProducers` bookkeeping — the real chain tracks all of this. Keep `RecentBlockProducers` replaced by a bounded `RecentAuthors` (last 100) fed from digests for entropy/telemetry.

## 6. Staking & validator set

- `bond(amount)`: uses `pallet_balances` **holds** (`RuntimeHoldReason`, e.g. `GhostStaking`) — funds stay on the account but unspendable. Min `MinStake = 1 * UNIT`.
- `bond_extra`, `unbond(amount)` → `UnbondingChunks` (bounded, e.g. `MaxUnbondingChunks = 16`), chunks unlock after `UnbondingPeriod` blocks (dev: 100; prod: `14 * DAYS`); `withdraw_unbonded` releases.
- `validate()`: opts a staker into the validator candidate set (requires `MinStake` + having set session keys via `pallet_session::set_keys`).
- `chill()`: leaves the candidate set.
- `pallet_session::SessionManager`: `new_session()` returns the top `MaxValidators` (e.g. 100, bounded sort of `CandidateList`) by stake. Sessions every `SESSION_PERIOD = 20` blocks (dev), GRANDPA authority set rotates with sessions via pallet_grandpa's `OneSessionHandler`.
- Session keys = GRANDPA key only (`impl_opaque_keys { grandpa }`).

## 7. Slashing

- **Equivocation**: `pallet_offences` + GRANDPA equivocation reports. Pallet implements `OnOffenceHandler` → slash `DoubleSignSlashPercentage` (100%) of bonded stake, mark validator chilled, append bounded `SlashingRecords`.
- **Downtime**: validators must submit `heartbeat()` once per session. At `new_session`, validators in the outgoing set with no heartbeat are slashed `DowntimeSlashPercentage` (10%) and removed from candidates. Bounded: iterates only the validator set (≤ MaxValidators), never all stakers.
- `report_misbehavior` extrinsic is deleted — equivocation flows through `pallet_grandpa::report_equivocation` → offences; downtime is automatic.
- All storage is bounded (`BoundedVec`, maps keyed by account, no unbounded `iter()` in hot paths — validator iteration capped by `MaxValidators`).

## 8. PQC (Dilithium5 / ML-DSA-87)

- `pqc` module inside `pallet-ghost-consensus` (new file `src/pqc.rs`).
- No_std verification via a pure-Rust verifier: first choice `pqc_dilithium` (0.5.x, no_std-capable, Dilithium5); fallback `ml-dsa` (RustCrypto). Verify `pqc_dilithium::verify(sig, msg, pk)` is deterministic and no_std; compile-gate behind the pallet's `no_std` path (no `#[cfg(feature="std")]` guards on verification — the point is it works in Wasm).
- `register_pqc_key(pk, pop)`: `pk: [u8; 2592]`, `pop: PqcSignature([u8; 4627])` = signature over `b"GHOST-PQC-POP" || account_id`; reject if verification fails. This is proof-of-possession — prevents key-registration DoS/rogue-key issues.
- `pqc_attest(block_hash, sig)`: bonded validator attests a finalized block hash with its registered PQC key; emits event. Attestations are informational (they do not gate finality in v1).
- `PqcRequired` constant deleted — verification is always on where used.
- Must be benchmarked; verification weight is substantial — `pqc_attest` gets its own measured weight, and extrinsic fees reflect it.

## 9. Node service wiring (`node/src/service.rs`)

- Import queue: `sc_consensus_pow::import_queue(PowBlockImport { algorithm: GhostPowAlgorithm, inner: grandpa_block_import, ... })` — PoW seal verification → GRANDPA justification handling → import.
- `select_chain`: a `HeaviestChain`-style `SelectChain` implementation (in `consensus/`) that picks the leaf with max `PowAux.total_difficulty`, tie-break higher block number, then lower hash. `sc_consensus::LongestChain` is WRONG here — it selects by number, while `PowBlockImport` decides the client's best by `ForkChoiceStrategy::Custom(total_difficulty)`. The same `select_chain` feeds GRANDPA's voter so finality tracks the heaviest chain.
- Mining: when `--validator`/`--miner` flag (new `RunCmd` flag `--mine` + `--mining-threads N` + `--miner-coinbase <SS58>`), spawn `start_mining_worker` + N grind threads reading `MiningHandle::metadata()` → `submit()`. `--dev` mines by default with Alice as coinbase.
- RPC: `ghost_getConsensusMode` reports real status from client (engine name, difficulty, peers); `ghost_getPqcStatus` reports registry state via runtime API.
- `ghost mine` CLI becomes a standalone CPU miner talking to a node's RPC (get metadata → submit seal) OR an in-node thread — choose in-node for v1, CLI command drives the same path via `--mine` on run.

## 10. Inherents & block validity

- Inherent data providers: `sp_timestamp::InherentDataProvider` only (no aura slot inherent).
- `PowBlockImport` calls `check_inherents` — timestamp must be ≥ parent and not > drift. Keep default `MAX_FULL_VERIFICATION_BLOCKS` semantics for `check_inherents_after`.

## 11. Genesis / chain spec

- `dev`: Alice sole authority (grandpa key) + Alice stake; difficulty = trivial.
- `local_testnet`: Alice + Bob validators with stakes + session keys; endowed dev accounts.
- `ghost-testnet` preset: real validator bootnodes, real ss58 prefix (register a unique one in ss58-registry later; for now document `42` default), no sudo in production preset (sudo stays for testnet, flagged for removal pre-mainnet).

## 12. Testing gates (all must pass before "testnet-ready")

1. `cargo test --workspace` green.
2. `scripts/e2e-local.sh` rewritten for the Ghost path: builds node, runs 2 miners + 2 validators, checks: peering, PoW blocks authored (verify seal digests present via RPC `chain_getHeader` digest logs), GRANDPA finality advancing, reward issuance on-chain (query `system_account` for miner/validator), restart+rejoin+finalize.
3. Multi-node soak: `scripts/soak.sh` — 4+ nodes, 30+ min, assert finalized-head agreement and no equivocation stalls.
4. Benchmarks run for every ghost extrinsic; weights file generated into `pallets/pallet-ghost-consensus/src/weights.rs` (checked in).
5. `cargo fmt --check`, `cargo clippy -- -D warnings` clean.
6. Docker build produces a working `ghost-node` image.
7. Security review pass: no unbounded storage iteration, no unverifiable slashing evidence, no integer overflow paths (saturating/checked everywhere), digest decoding bounds-checked.

## 13. Non-goals for v1

- Spectral/ZK off-chain transactions (mentioned in early notes) — out of scope.
- Replacing GRANDPA with a PQC finality gadget — research only.
- Nominated PoS / delegation — validators are self-staked only in v1.
