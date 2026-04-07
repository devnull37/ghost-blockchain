# Ghost Blockchain

Ghost is a Substrate-based chain prototype with an experimental `ghostConsensus` pallet.

## Current State

- Live node consensus is still `Aura` for block production and `GRANDPA` for finality.
- `pallet-ghost-consensus` implements a pallet-level hybrid PoW/PoS flow that can be exercised with extrinsics and unit tests.
- No quantum encryption or post-quantum finality is currently implemented.
- This repository is suitable for local development and verification, not a production launch.

## What Works

- Substrate node startup and development chain flow
- Pallet-level staking, block submission, validator selection, rewards, and slashing logic
- Local mining helper CLI for generating PoW nonces for the pallet model

## What Does Not Exist Yet

- A custom block authoring/import pipeline that replaces Aura/GRANDPA
- Production-grade validator economics, custody, and security review
- A real post-quantum cryptography path in the runtime or node

## Build

```sh
cargo build --bin ghost-node
```

## Test

```sh
cargo test -p pallet-ghost-consensus --target-dir .cargo-target
```

## Run A Local Node

```sh
cargo run --bin ghost-node -- --dev
```

## Ghost CLI Helpers

```sh
cargo run --bin ghost-node -- ghost status --detailed
cargo run --bin ghost-node -- ghost mine --threads 2
```

`ghost status` intentionally reports the real node/runtime state. It does not claim a custom network consensus engine or quantum encryption.

## Important Paths

- `node/src/service.rs`: actual node authoring/finality wiring
- `node/src/command.rs`: Ghost helper CLI output
- `runtime/src/lib.rs`: runtime composition
- `runtime/src/configs/mod.rs`: runtime pallet configs
- `pallets/pallet-ghost-consensus/src/lib.rs`: pallet logic
- `pallets/pallet-ghost-consensus/src/tests.rs`: pallet verification
