# RPC Surface Audit

Inventory of every JSON-RPC method `ghost-node` exposes and its safety
posture. Rule: **no method may mutate chain state, expose key material, or
perform unbounded work.**

## Custom methods (`node/src/rpc.rs`)

| Method | What it does | Safety |
|---|---|---|
| `ghost_getNodeStatus` | Node role/sync summary | read-only, bounded |
| `ghost_getConsensusMode` | Engine description + `next_difficulty` (via `GhostPowApi` runtime call at best hash) | read-only, bounded |
| `ghost_getPqcStatus` | Static description of the PQC registry path | read-only, constant |

All three are infallible-over-RPC (errors render as `<unavailable>` inside
the string, never as protocol errors) and do no storage iteration.

## Upstream methods

| Group | Exposure |
|---|---|
| `system_*` (via `substrate_frame_rpc_system`) | wired with `DenyUnsafe` honored — unsafe calls (`system_addReservedPeer`, `system_nodeRoles`, …) are blocked unless `--rpc-methods Unsafe` |
| `payment_*` (`TransactionPaymentApiServer`) | read-only fee query, safe |
| `author_*` (substrate core, not custom) | **unsafe surface** — `author_submitExtrinsic` is needed publicly, but `author_insertKey`/`author_rotateKeys` write/read keystore material |
| `chain_*`, `state_*` | read-only; `state_call`/large storage reads are bounded by the node |
| `grandpa_*` equivocation proofs | read-only |
| `offchain`, `dev`, `sync`, `childstate` | unsafe; blocked by default externally |

## Defaults and operator guidance

- **`--rpc-methods Auto` (default)**: unsafe methods are served to
  localhost only — correct default. Keystore methods (`author_insertKey`,
  `author_rotateKeys`) therefore work from the node's own box but not
  remotely.
- **`--rpc-methods Unsafe`**: exposes keystore and admin methods to every
  RPC client — never set it on a public node. Our own e2e scripts use it
  on throwaway local nodes only.
- **Public RPC nodes** should front the port with a proxy that allowlists
  `chain_*`/`state_*`/`system_health`/`author_submitExtrinsic` — the
  built-in safe/unsafe split is the first line, not the whole policy.
- No method anywhere returns secret material; keystore access is one-way
  (`insertKey` writes, `rotateKeys` generates internally).

## Client compatibility (verified live, spec-103 binary)

- `rpc_methods` reports 102 methods: the full legacy set PJS Apps /
  subxt need (`system_*`, `chain_*`, `state_*`, `author_*`, `payment_*`)
  plus spec-v2 (`chainHead_v1_*`, `transaction_v1_*`) for newer clients.
- `state_getMetadata` serves a complete v1x runtime metadata blob
  (~54 kB) — call-index/type registry intact for extrinsic builders.
- `system_properties` now serves `ss58Format`, `tokenDecimals`,
  `tokenSymbol` (was `{}` before; fixed) — wallets render balances
  correctly instead of raw planck.
- `ghost_*` methods are additive; nothing shadows upstream methods.

## Verdict

Surface is minimal by construction — three custom read-only methods,
`DenyUnsafe` plumbed through, no unbounded queries. The remaining risk is
operator-set `--rpc-methods Unsafe` on public nodes; covered by guidance
above and in `docs/operator-guide.md`.
