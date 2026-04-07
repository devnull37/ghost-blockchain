# Repository Notes

## Current Architecture

- The node runs `Aura` for authoring and `GRANDPA` for finality.
- `pallet-ghost-consensus` is an experimental runtime pallet that models a PoW header submission
  plus stake-weighted validation flow.
- There is no production-ready post-quantum or "quantum encryption" implementation in the chain.

## Useful Commands

```sh
cargo build --bin ghost-node
cargo test -p pallet-ghost-consensus
cargo run --bin ghost-node -- --dev
```

## Important Caveats

- Do not describe the node as using a custom Ghost block production engine unless `node/src/service.rs`
  is actually rewritten to replace Aura/GRANDPA.
- Do not claim PQC or quantum finality is active on-chain.
- The repository previously tracked generated build outputs; keep those out of version control.
