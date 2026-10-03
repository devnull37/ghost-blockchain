//! Ghost PoW consensus engine (client side).
//!
//! Implements the consensus layer specified in `docs/ghost-consensus-design.md`:
//! real proof-of-work block production and import on top of `sc-consensus-pow`.
//!
//! * [`GhostPowAlgorithm`] — the `PowAlgorithm` implementation the node's
//!   `PowBlockImport` / `PowVerifier` uses to check seals and resolve per-block
//!   difficulty (runtime API `GhostPowApi::next_difficulty` with an aux-storage
//!   fallback).
//! * [`HeaviestChain`] — `SelectChain` that follows `PowAux.total_difficulty`,
//!   feeding both `PowBlockImport` and the GRANDPA voter.
//! * [`mining`] — pure hash/grind helpers shared by the in-node `--mine` loop
//!   and by tests; both compute the exact digest `verify` checks.
//! * [`aux`] — `PowAux` persistence helpers so total difficulty accumulates
//!   across restarts.
//! * [`difficulty`] — the deterministic retarget math (`compute_next_difficulty`)
//!   mirrored by `pallet-ghost-consensus`.
//!
//! The wire types (`GhostSeal`, `POW_ENGINE_ID`, `GhostPowApi`) are defined in
//! `ghost-pow-primitives` and re-exported here so consumers only depend on this
//! crate.

mod algorithm;
mod aux;
mod difficulty;
mod mining;
mod select_chain;

pub use algorithm::{author_from_header, verify_seal, GhostPowAlgorithm};
pub use aux::{aux_key, read_aux, write_aux};
pub use difficulty::{compute_next_difficulty, RETARGET_INTERVAL, TARGET_BLOCK_TIME_MS};
pub use mining::{
    author_from_pre_digest, grind, hash_meets, miner_digest_item, miner_pre_runtime, pow_meets,
    pow_value,
};
pub use select_chain::HeaviestChain;

pub use ghost_pow_primitives::{GhostPowApi, GhostSeal, POW_ENGINE_ID};

// Re-export the sc-consensus-pow machinery the node wires up, so `node/`
// imports everything consensus-related from this one crate.
pub use sc_consensus_pow::{
    import_queue, start_mining_worker, Error, MiningBuild, MiningHandle, MiningMetadata,
    PowAlgorithm, PowAux, PowBlockImport, PowImportQueue, PowIntermediate, PowVerifier,
    INTERMEDIATE_KEY, POW_AUX_PREFIX,
};
pub use sp_consensus_pow::Seal;
