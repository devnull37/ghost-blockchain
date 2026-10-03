//! Shared Ghost PoW consensus primitives.
//!
//! This crate is the single source of truth for the wire format that the
//! node-side consensus engine (`sc-consensus-pow` machinery) and the runtime
//! (`pallet-ghost-consensus` + runtime API implementation) exchange:
//!
//! * [`GhostSeal`] — the proof-of-work seal carried in the header's
//!   `DigestItem::Seal(POW_ENGINE_ID, ..)` slot.
//! * [`POW_ENGINE_ID`] — the 4-byte engine identifier used for both the
//!   seal and the author pre-runtime digest.
//! * [`GhostPowApi`] — the runtime API the node calls to learn the difficulty
//!   target the next block must satisfy.
//!
//! Everything here must be `no_std`-compatible: the runtime API declaration
//! is compiled into the Wasm runtime.
#![cfg_attr(not(feature = "std"), no_std)]

use codec::{Decode, Encode};
use scale_info::TypeInfo;
use sp_core::U256;

/// Consensus engine identifier for Ghost proof-of-work headers.
///
/// Used twice per block, per `docs/ghost-consensus-design.md`:
/// * `DigestItem::PreRuntime(POW_ENGINE_ID, SCALE(AccountId32))` — carries the
///   miner's account, bound into the seal's hash input so authorship cannot be
///   spoofed.
/// * `DigestItem::Seal(POW_ENGINE_ID, SCALE(GhostSeal))` — carries the nonce.
pub const POW_ENGINE_ID: [u8; 4] = *b"pow_";

/// PoW seal attached to a block header.
///
/// The PoW hash is computed over `pre_hash ++ pre_digest ++ seal.encode()`
/// (double BLAKE2-256) and interpreted as a little-endian U256; the seal is
/// valid when that value does not exceed the per-block difficulty target
/// returned by [`GhostPowApi::next_difficulty`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Encode, Decode, TypeInfo)]
pub struct GhostSeal {
    /// Counter the miner incremented until the PoW hash met the target.
    pub nonce: u64,
}

sp_api::decl_runtime_apis! {
    /// Queries consensus state the node needs to author and validate blocks.
    pub trait GhostPowApi {
        /// Difficulty target the *next* block must satisfy, as decided by the
        /// runtime's retarget logic (`pallet-ghost-consensus` `on_initialize`
        /// every `RETARGET_INTERVAL` blocks toward the target block time,
        /// clamped to [prev/4, prev*4]).
        fn next_difficulty() -> U256;
    }
}
