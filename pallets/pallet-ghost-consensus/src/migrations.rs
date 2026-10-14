//! Storage migrations for the Ghost consensus pallet.
//!
//! v1 -> v2 clears every legacy storage item left by the simulation pallet:
//! the `submit_block`/`validate_block` bookkeeping, `ConsensusPhase`, per-block
//! header maps, extrinsic-reported misbehavior flags, and the old `u64`
//! `Difficulty` (v2 stores a `U256` work factor under the same name).

use super::{Config, Pallet};
use frame_support::{
    storage::migration::clear_storage_prefix,
    traits::{Get, GetStorageVersion, OnRuntimeUpgrade, PalletInfoAccess, StorageVersion},
    weights::Weight,
};
use sp_core::U256;
use sp_std::vec::Vec;

/// The on-chain storage version of this pallet.
pub const STORAGE_VERSION: StorageVersion = StorageVersion::new(2);

/// Legacy v1 single-value storage items (under `twox128(pallet) ++
/// twox128(item)`).
const LEGACY_VALUES: &[&[u8]] = &[
    b"Difficulty".as_slice(),
    b"CurrentPhase".as_slice(),
    b"SlashingRecords".as_slice(),
    b"RecentBlockProducers".as_slice(),
    b"CurrentEntropy".as_slice(),
];

/// Legacy v1 map storage items (whole map prefixes are killed).
const LEGACY_MAPS: &[&[u8]] = &[
    b"BlockHeaders".as_slice(),
    b"BlockProducers".as_slice(),
    b"ValidatorStakes".as_slice(),
    b"LastActiveBlock".as_slice(),
    b"DoubleSignReports".as_slice(),
    b"InvalidBlockReports".as_slice(),
    b"ValidatorPqcPublicKeys".as_slice(),
];

/// Clears all legacy v1 storage items and stamps `StorageVersion` 2.
///
/// Runs when the on-chain storage version is below 2. Safe to run on a chain
/// that never had v1 state (clears nothing). The cleared `Difficulty` (v1
/// `u64`) is re-initialized to `MinDifficulty` so the chain always has a valid
/// work factor after the upgrade.
pub struct MigrateToV2<T>(core::marker::PhantomData<T>);

impl<T: Config> OnRuntimeUpgrade for MigrateToV2<T> {
    fn on_runtime_upgrade() -> Weight {
        let on_chain = Pallet::<T>::on_chain_storage_version();
        if on_chain >= STORAGE_VERSION {
            return T::DbWeight::get().reads(1);
        }

        let pallet_prefix = Pallet::<T>::name().as_bytes();
        let mut removed: u64 = 0;

        for item in LEGACY_VALUES {
            let key = frame_support::storage::storage_prefix(pallet_prefix, item);
            frame_support::storage::unhashed::kill(&key);
            removed = removed.saturating_add(1);
        }

        for item in LEGACY_MAPS {
            // Unbounded removal: the prefix kill is a one-time migration on a
            // bounded legacy state; loop until the drain reports no cursor.
            let mut cursor: Option<Vec<u8>> = None;
            loop {
                let result =
                    clear_storage_prefix(pallet_prefix, item, &[], None, cursor.as_deref());
                removed = removed.saturating_add(u64::from(result.unique));
                cursor = result.maybe_cursor;
                if cursor.is_none() {
                    break;
                }
            }
        }

        // v1 `Difficulty` was a `u64`; after clearing, seed the v2 U256 work
        // factor at `MinDifficulty` when nothing valid remains.
        if super::Difficulty::<T>::get().is_zero() {
            super::Difficulty::<T>::put(T::MinDifficulty::get().max(U256::one()));
        }

        STORAGE_VERSION.put::<Pallet<T>>();
        T::DbWeight::get().reads_writes(removed.saturating_add(2), removed.saturating_add(2))
    }
}

#[cfg(test)]
mod migration_tests {
    // See tests.rs — `migration_clears_all_legacy_storage`.
}
