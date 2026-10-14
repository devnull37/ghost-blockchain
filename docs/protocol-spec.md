# Ghost Protocol Specification

Status: **implementer-facing specification of the Ghost consensus protocol as merged**.
Normative references, in order of authority on a conflict:

1. `primitives/ghost-pow/src/lib.rs` (`ghost-pow-primitives` crate) — wire types.
2. `consensus/ghost-consensus/src/*.rs` (`ghost-consensus` crate) — the client-side
   PoW engine (`GhostPowAlgorithm`, `HeaviestChain`, mining helpers, retarget math).
3. `pallets/pallet-ghost-pqc/src/lib.rs` — the merged ML-DSA-87 key registry.
4. `docs/ghost-consensus-design.md` — the ratified target architecture, for the
   parts not yet landed.

Sections describing behavior that is designed but not in merged code are marked
**[NOT YET IMPLEMENTED]**. On `devin/integration`, the live node still authors
with Aura and finalizes with GRANDPA (see `node/src/service.rs`); the
`ghost-consensus` and `ghost-pow-primitives` crates and `pallet-ghost-pqc` are
merged and unit/property-tested but not yet wired into `runtime/` or
`node/src/service.rs`. Nothing in this document may be cited as live behavior
unless the section says so.

## 1. Terminology and notation

- `++` is byte concatenation. `SCALE(x)` is the SCALE codec encoding of `x`.
- `U256` is `sp_core::U256`. `U256::MAX = 2^256 - 1`. SCALE encodes `U256` as
  32 little-endian bytes.
- `blake2_256` is unkeyed BLAKE2s-256 (equivalent to `hashlib.blake2s` with
  `digest_size=32` and default parameters).
- `POW_ENGINE_ID = b"pow_"` (`ghost-pow-primitives`), the `ConsensusEngineId`
  shared with `sp_consensus_pow`.
- `pre_hash` is the hash of a block header computed **after removing the
  trailing PoW `Seal` digest item** (see §2.3).
- `pre_digest` is the payload bytes of `DigestItem::PreRuntime(POW_ENGINE_ID, _)`.
- *Difficulty* means a **work factor**: larger = harder (kulupu convention).
  Never a target. Every ordering rule depends on this.

## 2. Block and header format

A Ghost block is a standard Substrate `generic::Block` with
`Header = generic::Header<BlockNumber, BlakeTwo256>` (`parent_hash`,
`number`, `state_root`, `extrinsics_root`, `digest`).

### 2.1 Digest items carried by a Ghost block

| Digest item | Position | Payload | Purpose |
|---|---|---|---|
| `PreRuntime(POW_ENGINE_ID, SCALE(AccountId32))` | before the seal; exactly one per header enforced by `find_pre_digest` (`MultiplePreRuntimeDigests` otherwise) | 32-byte miner account id | Author binding. Stamped by `start_mining_worker`'s `pre_runtime` parameter; covered by the seal hash (§3), so authorship cannot be repointed without re-winning the PoW. |
| `Seal(POW_ENGINE_ID, SCALE(GhostSeal { nonce }))` | **last item** — `PowVerifier::check_header` pops the final digest log and rejects if it is not this seal (`HeaderUnsealed` / `WrongEngine`) | `GhostSeal`: SCALE of `nonce: u64`, i.e. exactly 8 little-endian bytes | The proof of work itself. |
| `Consensus(GRANDPA_ENGINE_ID, ..)` | only on blocks that enact an authority-set change | scheduled/forced/ pause-change log | GRANDPA set rotation plumbing (inherited, unchanged). |
| `PreRuntime(..)` of other engines | none expected | — | Aura's `PreRuntime(AURA_ENGINE_ID, slot)` disappears with Aura **[NOT YET REMOVED — Aura is still the live author on devin/integration]**. |

### 2.2 Inherents

The block body must begin with the timestamp inherent
(`pallet_timestamp::Call::set`), the only inherent provider in the design
(`sp_timestamp::InherentDataProvider`); there is no Aura slot inherent once
Aura is removed **[NOT YET IMPLEMENTED — the live service still builds the
Aura slot inherent at `service.rs`]**. `PowBlockImport` runs `check_inherents`
against parent state with the default `MAX_FULL_VERIFICATION_BLOCKS` depth;
the timestamp must be ≥ parent's and within the node's allowed drift.

### 2.3 Pre-hash definition

`pre_hash = BlakeTwo256::hash(header')` where `header'` is the header with the
trailing `Seal(POW_ENGINE_ID, ..)` log removed. Concretely, `check_header`
pops **only** the last digest log; any `Seal` items deeper in `logs` remain
inside `pre_hash` (and are therefore hash-covered). Miners and verifiers both
hash the same `header'` — the miner because it builds the header before the
seal exists, the verifier because the seal was popped before hashing.

## 3. Proof of work

### 3.1 Seal type

```rust
pub struct GhostSeal { pub nonce: u64 }   // ghost-pow-primitives
```

`SCALE(GhostSeal)` is exactly the 8-byte little-endian nonce (asserted by
`seal_roundtrip` in `mining.rs`). On the wire `Seal = Vec<u8>` carries those
bytes raw.

### 3.2 Hash input and validity rule

Given:

- `pre_hash`: 32 bytes (§2.3),
- `pre_digest`: payload of the single `PreRuntime(POW_ENGINE_ID, _)` log —
  must SCALE-decode (prefix/streaming decode) to `AccountId32`. Trailing
  bytes, if any, are ignored for attribution but remain covered by the hash.
  A header whose pre-digest is missing or cannot decode is **invalid**
  (`author_from_pre_digest` → `verify_seal` returns false),
- `seal_bytes`: payload of the trailing `Seal(POW_ENGINE_ID, _)` log — must
  `decode_all` to `GhostSeal`; padded or truncated encodings are rejected,

the PoW hash is:

```
input  = pre_hash ++ pre_digest ++ seal_bytes            // 32 + 32 + 8 = 72 bytes (canonical case)
inner  = blake2_256(input)
outer  = blake2_256(inner)
value  = U256::from_big_endian(outer)
```

A block is PoW-valid at difficulty `d` (a work factor) iff

```
value <= floor(U256::MAX / d)        // d > 0
```

`d == 0` is never valid. This is the merged formulation
(`mining.rs::pow_meets`, `algorithm.rs::verify_seal`). It is equivalent to
`value * d <= U256::MAX` for all `d > 0`, and strictly safer at `d = 0`
(where the multiplicative form would accept every value).

Consequences an implementer can rely on:

- Expected grind length ≈ `d` hashes per valid block. (Success set size is
  `floor(MAX/d) + 1`, so `P ≈ 1/d`.)
- `pow_meets` is monotone: meeting a harder `d` implies meeting any easier one
  (property-tested in `mining.rs`).
- `preliminary_verify` returns `Ok(None)` — there is deliberately no stateless
  pre-check; difficulty resolution needs parent state (§4.1).

### 3.3 Worked example (verifiable by hand)

Using `pre_hash = 00 01 02 .. 1f` (32 bytes), `pre_digest = 2a`*32, work
factor `d = 64`, so `limit = floor(MAX/64) = 0x03ff..ff`:

| nonce | `outer = blake2_256(blake2_256(input))` | `value <= limit` |
|---|---|---|
| 0 | `22b4d1d93c6db23d55f306a75f614dc1ef8ebf7b393ef531108f4d61740ffbbd` | fail |
| 1 | `c359bd5e23c6a048244b45bb137fd9db626ba073b449722b3da9368ce1aa29a3` | fail |
| 2 | `320f5de7798fd4c76aa2ee6ed2b003430993977cb6aeb63bf47ffd25cc19b64c` | fail |
| 3 | `9c5204225090e9118ce6b56283904e0627c069758c11531ce67d772dbef73859` | fail |
| 4 | `677ff51a473fff82ea92a182d95c7c87b7dc6cfea9efc5523894ac36d0378b1c` | fail |
| 5 | `31502ceb8347facf5c1c1b912b4f67068a0a77b2dfcfa6315a7320659d684e3e` | fail |
| 6 | `5a7b62ab5423b3af7781a314b6bd29ee4121a4c568163db88a2f7b1ce99f1f38` | fail |
| 7 | `01a553c159511863fb475ac9f3395b3eeeb265c99d322c50c88134703b6d7600` | **pass** |

with `input = pre_hash ++ pre_digest ++ nonce.to_le_bytes()` =
`0001..1f` ++ `2a`*32 ++ `0700000000000000`. Reproduce with:

```python
import hashlib
inp = bytes(range(32)) + bytes([0x2a]*32) + (7).to_bytes(8, "little")
outer = hashlib.blake2s(hashlib.blake2s(inp, digest_size=32).digest(), digest_size=32).digest()
assert int.from_bytes(outer, "big") <= ((1 << 256) - 1) // 64
```

`verify_seal` is independently exercised in `algorithm.rs` tests at the exact
boundary (`d = MAX / value` passes, `d = MAX / value + 1` fails).

### 3.4 Mining path

`start_mining_worker` is driven with `pre_runtime = SCALE(AccountId32)` of the
coinbase account (`mining.rs::miner_pre_runtime`). Each new best block yields
`MiningMetadata { best_hash, pre_hash, pre_runtime, difficulty }`. Mining
threads call `grind(metadata, nonce_start = thread_id, nonce_step = num_threads,
rounds)` partitioning the nonce space by stride; `grind` refuses to produce a
seal when `metadata.pre_runtime` is `None` (it could never verify). A found
`GhostSeal` is submitted as `MiningHandle::submit(seal.encode())`, which
re-runs `verify` before import, so a grind result is guaranteed consistent
with the import rule by construction — both use `pow_value`.

## 4. Difficulty adjustment

### 4.1 Per-block resolution (merged: `GhostPowAlgorithm::difficulty`)

For each imported block the algorithm resolves the required difficulty of the
**next** block:

1. `GhostPowApi::next_difficulty(parent_hash)` — runtime API
   **[declaration merged in `ghost-pow-primitives`; runtime implementation
   NOT YET IMPLEMENTED]**.
2. Fallback: `PowAux` record of the parent (`aux.difficulty`, §5.1).
3. Fallback: configured `initial_difficulty` (must equal the genesis
   `Difficulty` the pallet will hold; today supplied as
   `GhostPowAlgorithm::new(client, initial_difficulty)` — the corresponding
   chain-spec field `ghostConsensus.difficulty` is **[NOT YET IMPLEMENTED]**).

A one-entry memo absorbs the two calls-per-import the `sc-consensus-pow`
pipeline makes (`PowBlockImport` + `PowVerifier`).

### 4.2 Retarget math (merged: `difficulty.rs::compute_next_difficulty`)

Every `RETARGET_INTERVAL = 100` blocks the work factor is recomputed toward
`TARGET_BLOCK_TIME_MS = 5_000`, i.e. `expected_ms = 500_000`:

```
lower  = prev / 4
upper  = prev * 4        (saturating)
scaled = prev * expected_ms / elapsed_ms   (saturating mul; checked div, else prev)
next   = clamp(scaled, lower, upper) .max(min)
```

Edge cases (exact, not approximate):

- `elapsed_ms == 0` → `next = prev * 4 .max(min)` (max-up retarget).
- `expected_ms == 0` → `next = prev` (degenerate; never divides by zero).
- `min` is the genesis floor (`MIN_DIFFICULTY`, ≥ 1) — a work factor of 0 would
  make every seal valid and corrupt `total_difficulty` accounting.

Direction: blocks faster than target (`elapsed_ms < expected_ms`) *increase*
the work factor; slower blocks decrease it. Unit + proptest coverage in
`difficulty.rs` pins direction, bounds, and the floor.

### 4.3 Runtime-side retarget **[NOT YET IMPLEMENTED]**

The merged pallet (`pallet-ghost-consensus`, pallet index 8) still carries the
prototype `Difficulty: u64` storage and a placeholder `adjust_difficulty`
that does **not** implement this formula (it hard-codes `actual_block_time =
5` and applies an entropy-steering term the design removed). The design's
runtime pieces — `Difficulty: U256`, `LastRetargetTime: Moment`,
`RetargetsDone: u32`, the `block_number % RETARGET_INTERVAL == 0` trigger in
`on_initialize`, and the `GhostPowApi::next_difficulty` implementation — are
pending the runtime-v2 workstream.

Timestamp caveat for that implementation: FRAME applies `on_initialize` hooks
**before** the timestamp inherent executes inside the block, so
`pallet_timestamp::Now` read during `on_initialize(n)` is the timestamp of
block `n - 1`. The retarget must therefore measure the window as
`Now(n-1) - LastRetargetTime` (i.e. timestamps of the *first and last completed
blocks* of the window), and tolerate the window being one block shorter than
`RETARGET_INTERVAL` in timestamp terms.

## 5. Fork choice

### 5.1 Per-block aux record (merged: `aux.rs`, `sc-consensus-pow`)

On every import `PowBlockImport` writes

```
PowAux { difficulty, total_difficulty }
```

to aux storage under key `b"PoW:" ++ block_hash` (`POW_AUX_PREFIX ++ hash`;
re-derived as `aux_key` since upstream keeps it private). `total_difficulty`
is the parent's total plus this block's work factor, via
`TotalDifficulty::increment` = **saturating add** on `U256`. Missing records
decode to zero, which is also how genesis behaves. `read_aux`/`write_aux`
exist for recovery tooling and tests; live writes come from the import path.

### 5.2 Import-time best-block decision (merged via `sc-consensus-pow`)

`PowBlockImport` sets `ForkChoiceStrategy::Custom` per block:
`new_total > best_total` → new best; `new_total < best_total` → keep; on
equality it defers to `algorithm.break_tie(best_seal, new_seal)`.
`GhostPowAlgorithm` does not override `break_tie`, so the default `false`
applies: **the earliest-seen equal-work leaf stays best**. Note this is
*per-import* — it marks which imported block becomes best, not which leaf a
later query returns.

### 5.3 `HeaviestChain` selection rule (merged: `select_chain.rs`)

For everything that asks "what is best" (RPC, and critically the GRANDPA
voter), `HeaviestChain` scans `backend.blockchain().leaves()` and orders by:

1. max `PowAux.total_difficulty` (zero for blocks with no aux — degrades to
   the next keys),
2. then max block number,
3. then **min** header hash (lexicographic byte order).

`leaf_beats` is a pure function and unit-tested for all three keys.
`finality_target(base, maybe_max_number)` walks ancestors from the heaviest
head — identical semantics to `LongestChain`'s implementation, but anchored
on the heaviest leaf, so GRANDPA votes track most-work rather than
most-blocks. `sc_consensus::LongestChain` must not be used for this chain.

## 6. Finality

**Design (§1/§9 of the design doc), partially merged:**

- GRANDPA finalizes the heaviest chain: `PowBlockImport` wraps the GRANDPA
  block import (seal check → justification handling → import), and the same
  `HeaviestChain` instance is passed to the GRANDPA voter so finality follows
  total difficulty.
- Committee = the top `MaxValidators` stakers by bonded stake, produced by
  `pallet_session`'s `SessionManager::new_session`; sessions every
  `SESSION_PERIOD = 20` blocks (dev); GRANDPA authority set rotates with
  sessions via `OneSessionHandler`; session keys = GRANDPA key only
  (`impl_opaque_keys { grandpa }`). **[NOT YET IMPLEMENTED — `pallet_session`,
  `SessionManager`, and session-key plumbing are not in the runtime; today's
  live chain uses a genesis-static GRANDPA set and Aura authoring.]**
- Equivocation: `pallet_grandpa::report_equivocation` → `pallet_offences` →
  slash `DoubleSignSlashPercentage` of bonded stake + chill.
  **[NOT YET IMPLEMENTED — the live runtime's `GrandpaApi` equivocation hooks
  return `None` and `pallet_offences` is not instantiated; the old pallet's
  `report_misbehavior` extrinsic is prototype bookkeeping, see §11.]**

## 7. Rewards and miner attribution

**Design rule (design doc §5):** per-block mint `BlockReward`, **40%** to the
block author decoded from `PreRuntime(POW_ENGINE_ID, SCALE(AccountId32))`,
**60%** pro-rata to the current session validator set
(`pallet_session::validators()`, stake-weighted); empty set → pallet reserve
account; rounding remainder to the author. Attribution is unspoofable because
the author digest is inside the seal's hash input (§3): forging it means
re-winning the PoW.

**Merged state:** `author_from_header` / `author_from_pre_digest` in the
consensus crate implement exactly this decoding (client side, unit-tested).
The on-chain mint path — pallet reading `frame_system`'s digest in
`on_finalize`/`on_initialize`, minting, and splitting — is
**[NOT YET IMPLEMENTED]**. The prototype pallet's `distribute_block_rewards`
(40/60 split, pro-rata over *all* `ValidatorStakes` entries, dust simply
unminted rather than returned to the author) exists but is only reachable
through its own extrinsic flow and is not authoritative (§11).

## 8. PQC layer (merged: `pallets/pallet-ghost-pqc`)

Merged, unit-tested, no_std throughout. **Not yet in the runtime** — it is a
workspace pallet awaiting the runtime-v2 composition.

- Scheme: **ML-DSA-87** (FIPS-204; the standardized form of Dilithium5),
  verified with the pure-Rust `ml-dsa` crate (`0.1`, `default-features=false`,
  no `alloc`/`getrandom`). `pqc_dilithium` was rejected: no 0.5 release exists
  and 0.2.0 is std-only (fails `wasm32`).
- Key = 2592 bytes (`PQC_PUBLIC_KEY_BYTES`), signature = 4627 bytes
  (`PQC_SIGNATURE_BYTES`); both bounded by `BoundedVec` limits
  (`MaxPqcKeySize`/`MaxSigSize`); wrong lengths fail decode.
- `register_pqc_key(public_key: Vec<u8>, proof_of_possession: Vec<u8>)` —
  requires an ML-DSA-87 signature over `b"GHOST-PQC-POP" || SCALE(account_id)`
  under the submitted key (`POP_DOMAIN`). One key per account;
  `KeyAlreadyRegistered` until `revoke_pqc_key` frees the slot. PoP defeats
  rogue-key and key-spam registration.
- `revoke_pqc_key()` — removes the caller's key.
- `pqc_attest(block_hash: Hash, signature: Vec<u8>)` — ML-DSA-87 signature
  over the raw 32-byte block hash; requires a registered key. v1 gates on key
  ownership only (bonded-validator gating is a stated TODO pending the
  consensus pallet's provider). Attestations are informational and do not
  gate finality.
- `set_pqc_required(required: bool)` — root only. `PqcRequired` is a storage
  flag (default `false`) read by downstream pallets through
  `PqcKeyProvider::{has_pqc_key, pqc_key}` to gate validator eligibility;
  it never gates verification itself, which is always on.
- Weights are Wasm-measured via `benchmark pallet` (`weights.rs`, autogenerated):
  `register_pqc_key` ~1.14 s and `pqc_attest` ~1.16 s ref_time — the ML-DSA-87
  verify dominates — plus ~7.2 KiB call data; both far inside `MAXIMUM_BLOCK_WEIGHT`.

**Legacy contrast:** the old pallet's `register_pqc_key` stores a key with no
PoP and its `verify_pqc_signature` is `#[cfg(feature = "std")]`-gated
`pqcrypto_dilithium` (always `false` in Wasm). The registry above supersedes it.

## 9. Chain parameters

| Parameter | Value | Source of truth |
|---|---|---|
| Engine id | `b"pow_"` | `ghost-pow-primitives` (merged) |
| Hash function | double BLAKE2s-256 | `mining.rs::pow_value` (merged) |
| Difficulty type | `U256` work factor | `algorithm.rs` (merged) |
| Retarget interval | 100 blocks | `RETARGET_INTERVAL` (merged crate; pallet pending) |
| Target block time | 5 000 ms (`expected_ms = 500_000`) | `TARGET_BLOCK_TIME_MS` (merged crate; pallet pending) |
| Retarget clamp | `[prev/4, prev*4]` per retarget | `compute_next_difficulty` (merged) |
| Min difficulty | genesis floor ≥ 1 | `min` arg (merged fn; `MIN_DIFFICULTY` const pending) |
| Block reward | 10 UNIT (`10_000_000_000_000` base units) | runtime config (prototype pallet; live-issuance path pending) |
| Reward split | 40% author / 60% validators pro-rata | design + prototype; digest attribution pending |
| Min stake | 1 UNIT | runtime config (prototype pallet) |
| Session period | 20 blocks (dev) | design only — `pallet_session` pending |
| Committee size | `MaxValidators = 100` | design only |
| Unbonding | 100 blocks (dev) / `14 * DAYS` (prod intent) | design only |
| Session keys | GRANDPA only (post-Aura) | design only — live runtime still `{aura, grandpa}` |
| Timestamp `MinimumPeriod` | 2 500 ms (`SLOT_DURATION / 2`) | runtime config (merged, lives behind Aura today) |
| Slash percentages | double-sign 100% / invalid-block 50% / downtime 10% | runtime config (prototype pallet; offences path pending) |
| Max downtime | 100 blocks | runtime config (prototype pallet) |
| PQC scheme | ML-DSA-87, key 2592 B / sig 4627 B | `pallet-ghost-pqc` (merged, not in runtime) |
| GRANDPA justification period | 512 blocks | `node/src/service.rs` (merged/live) |

## 10. Encoding and determinism rules

- All consensus-relevant values SCALE-encode: `GhostSeal` = 8-byte LE nonce;
  `AccountId32` = raw 32 bytes; `U256` = 32-byte LE.
- Seal decoding is strict (`decode_all` — no trailing bytes); author pre-digest
  decoding is prefix-style (`AccountId32::decode` — trailing bytes permitted
  but hash-covered). This asymmetry is deliberate and tested.
- `GhostPowApi` is declared via `sp_api::decl_runtime_apis!` (unversioned
  declaration = API version 1). Adding methods later requires a version bump
  of the API, not a runtime upgrade hack. **[runtime implementation pending]**
- `total_difficulty` accumulation saturates — two forks can never overflow
  their way into equal work at different heights.

## 11. Legacy simulation pallet (`pallet-ghost-consensus`, index 8)

Still compiled into the live runtime pending the v2 rewrite; **informational
bookkeeping only** — the node never consults it for authoring or import. Its
extrinsics (`submit_block`, `stake`, `unstake`, `validate_block`,
`report_misbehavior`, `register_pqc_key`) simulate the PoW→PoS→finalization
flow and prototype economics (`stake` escrows via plain transfer to the
pallet account, not holds; `SlashingRecords` is `#[pallet::unbounded]`;
`LastActiveBlock`/`ValidatorStakes` are iterated unboundedly in hooks and
dispatchables; `consensus.rs`/`rpc.rs` in that directory are dead files not
declared as modules). Treat any state it records as non-normative. None of
its storage feeds the real PoW rule.

## 12. Implementation status summary

| Component | Merged | Live in runtime/service |
|---|---|---|
| `ghost-pow-primitives` (seal, engine id, API decl) | yes | API not yet implemented |
| `ghost-consensus` engine crate | yes | not yet wired (`service.rs` unchanged) |
| `pallet-ghost-pqc` | yes | not in `construct_runtime!` |
| Runtime v2 (U256 difficulty, retarget, digest rewards, sessions) | — | pending workstream |
| Service wiring (PoW import queue, `HeaviestChain`, `--mine`) | — | pending workstream |
| Aura removal | — | pending workstream |

Until the pending rows land, every "chain" claim about Ghost PoW should cite
this table. The launch gate lives in `docs/testnet-readiness.md`.
