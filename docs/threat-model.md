# Ghost Threat Model

Status: analysis of the **target** Ghost protocol (`docs/ghost-consensus-design.md`)
plus the merged crates on `devin/integration` (`consensus/ghost-consensus`,
`primitives/ghost-pow`, `pallets/pallet-ghost-pqc`). The live chain today is
Aura/GRANDPA with a prototype pallet — attacks against the Ghost PoW design are
assessed on the merged code and the ratified design, and each section states
what is merged, what is pending, and what residual risk remains. Nothing here
is asserted without a code or design reference.

## 1. Scope and security goals

Protected properties:

1. **Consensus integrity** — an attacker must not be able to produce a valid
   Ghost block without expending ~`d` hashes (§3 of `protocol-spec.md`), and
   must not repoint block authorship (miner reward theft).
2. **Fork-choice integrity** — the canonical chain is the one with the most
   accumulated work; cheap production of an alternative best chain must not
   exist.
3. **Finality** — GRANDPA finality must track the heaviest chain and be
   attributable to the stake committee.
4. **Liveness** — difficulty, rewards, or slashing bugs must not stall block
   production or grind the work factor to 0/`U256::MAX`.
5. **Registry integrity** — PQC keys must not be registerable by anyone other
   than their owner (rogue-key/key-spam prevention); attestations must be
   unforgeable.
6. **State-machine integrity** — no unbounded iteration, overflow, or
   consensus-relevant nondeterminism.

Out of scope for v1 (design doc §13): PQC finality, delegated/nominated PoS,
ZK off-chain transactions, and collusion inside the honest-majority-of-stake
assumption.

## 2. Adversary classes

| ID | Adversary | Capability ceiling |
|---|---|---|
| A1 | Single miner, minority hash | Controls own mining output; can withhold, delay, or selectively publish blocks; can pick coinbase and any winning nonce. |
| A2 | Majority-hash attacker | ≥51% of network hash; can out-work the public chain privately. |
| A3 | Malicious validator subset | Up to `f` of the stake committee; can equivocate GRANDPA votes, withhold votes, or coordinate downtime. |
| A4 | Eclipse/partition attacker | Splits the network graph; controls message delivery to a victim set but no extra hash. |
| A5 | Runtime/extrinsic exploiter | Sends arbitrary extrinsics; looks for unbounded iteration, panics, bookkeeping-only slashing, unpriced PQC verification. |
| A6 | PQC-layer adversary | Registers/attests PQC material; aims for key substitution, registry spam, or verification-cost DoS. |

## 3. Assumptions (stated, not argued)

- **H1. Hash strength.** BLAKE2s-256 behaves as a random oracle for
  preimage/second-preimage at the 128-bit level. The PoW input is
  domain-separated only implicitly (by `pre_hash`/`pre_digest` structure); no
  additional domain tag is applied to the PoW hash itself.
- **H2. Honest stake majority for finality.** GRANDPA safety requires
  <1/3 malicious voting power in the authority set. With a genesis-static set
  (today's live chain) this is an *operational* assumption about the chosen
  authorities; with stake-selected committees (pending) it becomes an
  economic assumption about `MinStake` and stake distribution.
- **H3. Clock sync.** Nodes run roughly synchronized clocks; the timestamp
  inherent enforces monotonicity and bounded drift, but miners set the value.
- **H4. One PreRuntime per header.** The import path rejects headers with two
  `PreRuntime(POW_ENGINE_ID, _)` logs (`MultiplePreRuntimeDigests`), so
  authorship is unambiguous.
- **H5. Client honesty of aux.** `PowAux` is locally accumulated; a node trusts
  its own aux store. Corrupted aux → wrong fork choice locally; it does not
  propagate to peers.

## 4. Per-attack analysis

### 4.1 PoW forgery / cheap block production (A1, A2)

**Attack.** Mint a block without doing `~d` hashes, or re-stamp an existing
block.

**Mitigations (merged).** `verify_seal` requires (a) trailing
`Seal(POW_ENGINE_ID, ..)` that `decode_all`s to `GhostSeal`, (b) a single
`PreRuntime` whose payload decodes to `AccountId32`, (c) `value <= MAX/d` on
`blake2_256²(pre_hash ++ pre_digest ++ seal)`. The seal hash covers the
pre-digest, so authorship is bound into the proof. `check_header` pops the
last log and demands the PoW seal specifically — a GRANDPA seal or missing
seal fails closed (`WrongEngine`/`HeaderUnsealed`).

**Residual risk.** Seal decoding is strict but pre-digest decoding is
prefix-style: a miner may append arbitrary trailing bytes to its own
pre-digest; they are hash-covered but invisible to `author_from_pre_digest`.
Impact is limited to the miner's own block — it cannot change who the decoded
author is, only pad the proof input. Noted for audit, not currently
exploitable into reward theft. **[open]**

**Planned.** Seal-forgery e2e (production-plan P0: "invalid seals rejected
deterministically across nodes"), fuzz target for seal/digest decode (P0).

### 4.2 Deep reorg / 51% rewrite (A2)

**Attack.** Private heavier branch overtakes the public chain.

**Mitigations (merged).** Fork choice is total work, not length
(`ForkChoiceStrategy::Custom` at import; `HeaviestChain` for queries and
GRANDPA). The work factor is monotone in real work by construction, so the
only way to out-accumulate is to out-hash. `total_difficulty` saturates, so
overflow cannot manufacture equal-work ties at lower cost.

**Residual risk.** **High by design on a young chain**: until hashrate is
distributed and difficulty is non-trivial, a single larger miner rewrites
history cheaply. GRANDPA bounds the blast radius — *finalized* blocks are not
reorged — but unfinalized heads are fair game, and finality itself stalls if
the honest committee's heaviest chain keeps flipping.
**[inherent to PoW; mitigated only by finality + hashrate diversity]**

**Planned.** Finality over the PoW chain (P0), stake committee rotation (P0),
multi-miner contention e2e (P0).

### 4.3 Selfish mining / block withholding (A1, A2)

**Attack.** Withhold solved blocks to waste honest work, publish
strategically.

**Mitigations (merged).** `break_tie` stays `false`: on equal total work the
earliest-seen leaf wins, so a withheld equal-work block gains no tie-break
advantage from late release. There is no uncle/reward smoothing, so withheld
blocks risk becoming pure loss.

**Residual risk.** Classic selfish-mining profitability (Eyal–Sirer) is *not*
fully neutralized by first-seen tie-breaks under propagation advantage; the
dedicated selfish-mining analysis is a P1 production-plan item. A
block-hash-minimization tie-break (§5.3 rule 3) is deterministic and lets a
miner choose among its own winning seals — each winning seal yields a
different block hash, so `K` winning seals let the attacker lower its leaf
hash by ~factor `K` in ties. Bounded: it only helps within
equal-work/equal-number ties, but it is honest to note it is
attacker-influenceable at the margin. **[open, low]**

**Planned.** Selfish-mining exposure analysis in production plan P1.

### 4.4 Difficulty manipulation (A1, A5)

**Attack.** Force difficulty toward 0 (free blocks) or `U256::MAX` (stall), or
game the retarget via timestamps.

**Mitigations (merged).** `pow_meets` fails closed at `d = 0`
(`checked_div` → `None` → reject) — a zero work factor can never produce a
valid block, which also protects `total_difficulty` accounting. The retarget
clamps every interval to `[prev/4, prev*4]` and floors at `min >= 1`:
manipulating timestamps moves the work factor at most 4× per 100 blocks.
`elapsed_ms == 0` is a defined max-up retarget, not a panic or reset.
`expected_ms == 0` returns `prev` — the degenerate config can freeze the
retarget but cannot zero it.

**Residual risk.** The retarget consumes *miners' claimed timestamps*. Within
drift tolerance an attacker can consistently bias `elapsed_ms` to nudge
difficulty down over many windows (slow bleed); the clamp caps the rate, not
the direction. Mitigating requires honest timestamps (H3) — standard PoW
limitation. On the client side, `difficulty()` falls back to parent aux, then
to `initial_difficulty`; a runtime-API failure therefore cannot zero the
difficulty — worst case it freezes it at the last-good value.
**[bounded, open]**

**Planned.** Retarget unit/property tests exist in the merged crate; the
runtime-side retarget and its `on_initialize` timestamp source
(previous-block timestamp — see protocol-spec §4.3) are pending.

### 4.5 Author spoofing / reward theft (A1, A5)

**Attack.** Take another node's block, or claim its reward.

**Mitigations (merged).** The `PreRuntime` payload is inside the PoW input —
repointing authorship invalidates the seal. `find_pre_digest` rejects two
author claims. Reward attribution (pending) decodes the *same* digest bytes,
so rewards pay the account the PoW was actually won for.

**Residual risk.** None identified in the merged rule; the pending risk is the
*reward path itself* — the prototype pallet's extrinsic-based bookkeeping
(`BlockProducers` set by `submit_block`) is trivially spoofable if ever wired
to real issuance. **[closed in design; blocked on runtime v2]**

**Planned.** Digest-based reward path (P0); on-chain accounting tests (P0).

### 4.6 Equivocation / nothing-at-stake (A3)

**Attack.** Committee members vote on competing branches at no cost.

**Mitigations.** None merged yet: the runtime's `GrandpaApi` equivocation
hooks return `None`, `pallet_offences` is not instantiated, and the design's
`OnOffenceHandler` → slash-100%-of-bond + chill path is unbuilt. The old
pallet's `report_misbehavior` does extrinsic-reported slashing without
evidence — unusable as real mitigation (and itself an attack surface, §4.9).
**[open, high — depends on session-committee workstream]**

**Planned.** Offences pipeline + `DoubleSignSlashPercentage` slash + chill
(P0); equivocation-injection failure test (P0).

### 4.7 Finality stall via committee downtime (A3, A4)

**Attack.** >1/3 of voting power offline or partitioned → GRANDPA stalls.

**Mitigations.** Design-level: downtime heartbeats per session + automatic
slash/rotation out of the committee (bounded to validator set). Nothing
merged. **[open]**

**Residual risk.** On a small committee (dev presets: 1–2 authorities), one
offline validator is already a finality stall. Acceptable for devnet, not
testnet.

### 4.8 Eclipse / partition of miners (A4)

**Attack.** Isolate a miner subset; fork production locally per partition.

**Mitigations.** Standard Substrate networking (peer store, reserved nodes,
bootnodes); heaviest-chain rule guarantees convergence *after* healing —
partitions only delay, not fork, finality. **Residual:** during a partition,
both sides keep mining and the smaller-work side is orphaned on heal — no
worse than baseline PoW. **[accepted]**

### 4.9 Runtime/extrinsic abuse (A5)

**Attack.** Use callable surface to corrupt consensus bookkeeping or burn
block weight.

**Merged-code findings (prototype pallet, still in the live runtime):**

- `report_misbehavior` lets *any signed origin* slash any account's
  `ValidatorStakes` entry and set `DoubleSignReports`/`InvalidBlockReports`
  with no evidence — griefing of pallet bookkeeping. Mitigated only by the
  pallet being non-authoritative today; it must not gate real funds until
  removed (design deletes it in favor of offences-driven slashing).
- `check_downtime_slashing` iterates `LastActiveBlock` unboundedly every 10
  blocks; `SlashingRecords` is `#[pallet::unbounded]` — both are weight/state
  growth risks a future rewrite must bound.
- `ValidatorStakes::iter()` inside `validate_block`/`distribute_*` scales with
  staker count inside a dispatchable — same class of issue.
- `stake`/`unstake` use plain transfers to the pallet account (no hold reason,
  no unbonding delay) — instant unstake would let a slashed account front-run
  a slash if this pallet were ever authoritative.

**Mitigations pending.** The design replaces all of the above with bounded
structures (`MaxUnbondingChunks`, `RecentAuthors`), holds, and offence-driven
slashing. **[all open — runtime v2]**

### 4.10 PQC registry abuse (A6)

**Attack.** Register keys not yours; spam the registry; weaponize ~10 ms
verifies.

**Mitigations (merged, `pallet-ghost-pqc`).** PoP signature over
`b"GHOST-PQC-POP" || SCALE(account_id)` proves secret-key ownership at
registration — kills rogue-key registration and trivial key-spam. One key per
account with explicit `revoke`. Both `public_key` and `signature` are
length-checked `BoundedVec`s; ML-DSA-87 verify is constant-work per call.
`pqc_attest` requires a registered key and verifies against it; attestations
are informational and do not gate finality, so a bad attest cannot stall
consensus.

**Residual risk.** Verification is expensive on-chain (~10 ms placeholder
weight, unbenchmarked): `pqc_attest`/`register_pqc_key` are block-weight cost
vectors until real weights land. No storage deposit yet — registry growth is
price-capped only by extrinsic fees (a stated TODO). `pqc_attest` signs the
bare block hash with no domain tag — cross-protocol reuse is an implementer
hazard rather than an on-chain flaw, but a domain-separated attest message is
the cheap fix if attestation is ever security-relevant. **[open, low-medium]**

### 4.11 Long-range / weak subjectivity (A2, A4)

**Attack.** Present a syncing node with an alternative history from genesis.

**Mitigations.** PoW chains have no native weak-subjectivity from keys, but
GRANDPA finality gives effective checkpoints: a node that has seen a
finalized head rejects reorgs below it. Warp-sync proof provider is wired in
the live service (`service.rs`), and the pending PoW wiring keeps the
provider around the same finality gadget. **Residual:** a node syncing with
no finalized-head knowledge trusts heaviest-chain alone — inherits 4.2's
risk. **[partially mitigated]**

## 5. Known-gaps register

| Gap | Attack surface | Production-plan item |
|---|---|---|
| PoW path not live in service | all PoW rows above are crate-level, not chain-level | P0 consensus core |
| No `GhostPowApi` runtime impl / U256 retarget | difficulty freeze falls back to aux; placeholder pallet math unrelated | P0 retarget determinism |
| Digest-based rewards not wired | prototype `BlockProducers` is spoofable | P0 attribution/rewards |
| No session committee / offences / equivocation path | §4.6–4.7 | P0 committee + slashing |
| Unbounded iteration + extrinsic slashing in prototype pallet | §4.9 | P0 bounded storage review |
| Selfish-mining exposure unquantified | §4.3 | P1 threat-model item |
| ~~PQC weights are placeholders~~ → resolved: CLI-measured weights checked in | §4.10 block-weight DoS | done (`benchmark pallet`, spec-103 tree) |
| `pqc_attest` lacks domain tag + bonded gate | §4.10 | pallet TODO → P1 |
| Multi-winning-seal tie-break influence | §4.3, low | noted for audit |
| Sudo + template pallet in production presets | governance/griefing surface | P1 removal plan |

## 6. Security posture statement

The merged consensus crate correctly implements the PoW validity rule,
work-factor difficulty, heaviest-chain selection, and aux persistence; the
merged PQC pallet correctly implements ML-DSA-87 PoP registration and
attestation. The *chain* they describe is not yet the chain that runs. All
residual risks concentrated in §4.6–§4.10 trace to the pending runtime-v2 and
service-wiring workstreams — which is exactly what the production plan's P0
list tracks. This document is honest only as long as it keeps saying so.
