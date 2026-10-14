# Light-client story

Ghost exposes the standard Substrate light-client surfaces. Trust-minimized
sync works today through two mechanisms already wired into the node:

## Warp sync (state-download shortcut)

`node/src/service.rs` registers `warp_proof::NetworkProvider` as the warp
sync provider (`WarpSyncParams::WithProvider`). Full nodes serve GRANDPA
justification proofs so a light client (or a bootstrapping full node) can
jump to the latest finalized epoch without replaying history:

- The provider serves proofs keyed to the session-boundary committee set —
  exactly the validator set `pallet_ghost_consensus` + `pallet_session`
  rotate each 20-block session.
- Light clients trust a warp sync when the proof chain validates against
  the genesis GRANDPA authority set and each authority-set change is
  justified — there is no trusted checkpoint on our side.

## What a light client CANNOT do today

- **PoW header verification is cheap**: a light client can verify the
  `pow_` seal (single Blake2 hash ≤ `U256::MAX / difficulty`) from headers
  alone — no trusted node needed for production validity.
- **Fork choice is server-side**: `HeaviestChain` accumulates difficulty in
  client storage (aux store), which light clients don't replicate. A light
  client follows GRANDPA finality instead — the correct trust model for
  this chain (finality = committee-signed, PoW = production ordering only).
- **State proofs**: standard Substrate `state_getReadProof` works for
  specific storage keys; there is no dedicated light-client RPC surface
  beyond the spec-v2 `chainHead_*` methods the node already serves.

## Operator guidance

- Boot nodes serving warp sync need `--sync full` history back to at least
  the last authority-set boundary they want to prove; archival nodes are
  the right public sync servers (see `docs/node-ops.md`).
- A dedicated smoldot recipe is a P2 nicety — the trust model above is what
  such a recipe would encode.
