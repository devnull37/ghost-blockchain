# External audit engagement plan

Production bar requires eyes outside this repo. This doc scopes what an
external audit would cover and the current self-review depth it builds on.

## Self-review depth achieved (pre-audit baseline)

- Two structured review rounds landed as PRs (round 1: migration wiring,
  slash-immunity-via-unbond, `FindAuthor` starvation, fork-choice
  determinism; round 2 documented in `docs/security-review/`).
- `docs/threat-model.md` covers adversary classes; `docs/adversarial-report.md`
  documents the live fault-injection results (e2e-faults + forged-seal gates).
- `docs/security-advisories.md` is the accepted-risk register for the 37
  pinned-dependency advisories with exposure analysis and revisit policy.

## Scope for an external engagement (priority order)

1. **`consensus/ghost-consensus` + PoW fork choice** — seal parsing,
   `HeaviestChain` aux ordering, tie-break determinism across nodes.
2. **`pallet-ghost-consensus`** — staking/bond lifecycle, unbonding
   bounds, session selection top-N, retarget math (`clamp 1/4..4`),
   reward split accounting, `MigrateToV2` idempotence, offence handling.
3. **Runtime composition** — pallet indices, origin plumbing (governance
   stack: council thresholds vs democracy sovereign path), weight wiring,
   `Executive` migration order.
4. **`pallet-ghost-pqc`** — ML-DSA-87 PoP verification path, registry
   bounds, `RequirePqcKey` gate.
5. **Node/service** — `--mine` path, miner-coinbase plumbing, import
   queue deadlock surface, RPC surface (`docs/rpc-surface.md`).

## Candidate vendors / timing

- Firms with Substrate experience: SR Labs, Trail of Bits, Oak Security,
  Zellic, Quarkslab (PQC experience a plus for item 4).
- Engage **after** the SDK branch upgrade (P2: clears the advisory
  register and changes the reviewed code the least) and **before** the
  first public incentivized testnet epoch.
- Expected depth: 3–4 engineer-weeks for scope above; deliverable =
  findings report + fix-verification round, tracked as issues.

## What we hand them

- `docs/ghost-consensus-design.md`, `docs/protocol-spec.md`,
  `docs/threat-model.md`, this doc, the two review rounds, and the e2e
  gate suite (each gate is a reproducible adversarial scenario an auditor
  can extend).
