# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Ghost is a next-generation blockchain built on Substrate (Polkadot SDK) that implements a hybrid Proof-of-Work (PoW) and Proof-of-Stake (PoS) consensus mechanism. It combines PoW security with PoS energy efficiency for 5-second block times.

**Key Specifications:**
- Block Time: 5 seconds
- PoW Algorithm: Enhanced Blake2-256 (ASIC-resistant, double-hashed)
- Token: Ghost (GHTM)
- Block Reward: 10 GHOST per block (40% to miner, 60% to stakers)
- Minimum Stake: 1 GHOST token
- Built on: Polkadot SDK stable2407 branch

## Build Commands

**Prerequisites:**
- Rust toolchain must be installed via rustup
- Run `rustup default stable` if cargo is not found
- The project uses rust-toolchain.toml to configure stable Rust with wasm32-unknown-unknown target

**Build:**
```bash
# Release build (recommended for production)
cargo build --release --bin ghost-node

# Debug build (faster compilation for development)
cargo build --bin ghost-node

# Build documentation
cargo +nightly doc --open
```

**Testing:**
```bash
# Run all tests
cargo test

# Run tests for specific pallet
cargo test -p pallet-ghost-consensus

# Run a single test
cargo test test_name

# Run tests with output
cargo test -- --nocapture
```

**Running the Node:**
```bash
# Start development chain
./target/release/ghost-node --dev

# Start with custom base path
./target/release/ghost-node --dev --base-path ./ghost-chain-data

# Start with detailed logging
RUST_BACKTRACE=1 ./target/release/ghost-node -ldebug --dev

# Purge development chain state
./target/release/ghost-node purge-chain --dev
```

**Ghost-Specific CLI Commands:**
```bash
# Check consensus status
./target/release/ghost-node ghost status --detailed

# Start mining (when implemented)
./target/release/ghost-node ghost mine --threads 4

# Stake tokens for validation
./target/release/ghost-node ghost stake --amount 1000

# Check balance
./target/release/ghost-node ghost balance
```

## Architecture

### Workspace Structure

The project is a Cargo workspace with three main components:

**1. Node (`node/`)** - The blockchain client
- `cli.rs`: CLI structure including Ghost-specific commands (mine, stake, balance, status)
- `service.rs`: Node service configuration with Ghost consensus engine integration
- `chain_spec.rs`: Chain specification and genesis configuration (uses Alice/Bob as default validators)
- `rpc.rs`: RPC endpoint configuration

**2. Runtime (`runtime/`)** - The blockchain's state transition function (STF)
- `lib.rs`: FRAME runtime configuration with pallet composition
- `configs/mod.rs`: Pallet configurations
- Runtime uses 5-second block times (MILLI_SECS_PER_BLOCK = 5000)
- All pallets are configured via `impl $PALLET_NAME::Config for Runtime` blocks
- Composed into single runtime via `#[runtime]` macro

**3. Pallets (`pallets/`)** - Modular blockchain logic components

### Ghost Consensus Pallet (`pallets/pallet-ghost-consensus/`)

This is the core innovation of the project. It implements the hybrid PoW+PoS consensus.

**Key Files:**
- `src/lib.rs`: Main pallet with storage items, events, dispatchables, and hooks
- `src/types.rs`: Core data structures (GhostBlockHeader, ConsensusPhase, SlashingReason, etc.)
- `src/functions.rs`: Consensus algorithms (difficulty adjustment, PoW verification, validator selection)
- `src/consensus.rs`: Consensus engine integration with Substrate

**Storage Items:**
- `Difficulty<T>`: Current mining difficulty
- `CurrentPhase<T>`: Current consensus phase (PowMining, PosValidation, Finalization)
- `BlockHeaders<T>`: Block headers storage
- `ValidatorStakes<T>`: Validator stake amounts
- `LastActiveBlock<T>`: Tracks validator activity for slashing

**Consensus Flow:**
1. **PoW Phase**: Miners compete using enhanced Blake2-256 (double-hashed for ASIC resistance)
2. **PoS Phase**: Validators selected by weighted stake sign blocks
3. **Finalization**: Block rewards distributed (40% miner, 60% stakers)
4. **Slashing**: Penalties for double-signing, invalid blocks, and downtime

**Verification Functions:**
- `verify_pow()`: Basic Blake2-256 PoW verification
- `verify_pow_enhanced()`: Double-hashed Blake2-256 (current implementation)
- `verify_pow_sha256()`: Alternative SHA-256 verification

### Substrate Integration

The project uses Substrate's FRAME (Framework for Runtime Aggregation of Modularized Entities):

- **Pallets**: Self-contained modules with storage, events, errors, and dispatchables
- **Config Trait**: Each pallet has a Config trait for generic type/parameter configuration
- **Runtime**: Combines all pallets via macro-based composition
- **Dependencies**: All Substrate/Polkadot dependencies pinned to stable2407 branch

## Development Notes

**Default Development Accounts:**
- Chain uses Alice and Bob as default validator authorities
- Alice is the default sudo account
- Genesis state includes pre-funded development accounts (see `node/src/chain_spec.rs`)

**Consensus Configuration:**
- Uses Aura (Authority Round) for block authoring in dev mode
- Uses GRANDPA for finality
- Ghost consensus engine layers hybrid PoW+PoS on top

**Important Constants:**
- `BLOCK_HASH_COUNT`: 2400
- `GRANDPA_JUSTIFICATION_PERIOD`: 512 blocks
- Block time constants: `MINUTES`, `HOURS`, `DAYS` based on 5-second blocks

**Linting:**
- Workspace enforces `unsafe_code = "forbid"`
- Clippy warnings enabled for absolute paths, redundant lifetimes, and explicit outlives

## Testing with Polkadot-JS Apps

Connect to local node: https://polkadot.js.org/apps/#/explorer?rpc=ws://localhost:9944

Source code for hosting your own: https://github.com/polkadot-js/apps

## Multi-Node Testing

For testing consensus across multiple nodes, see Substrate docs on simulating networks:
https://docs.substrate.io/tutorials/build-a-blockchain/simulate-network/

## Common Issues

If you encounter "rustup could not choose a version of cargo":
```bash
rustup default stable
```

The project expects Rust stable toolchain with wasm32-unknown-unknown target (configured in `env-setup/rust-toolchain.toml`).
