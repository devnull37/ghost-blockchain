//! Weights for `pallet-ghost-consensus`.
//!
//! These are conservative, operation-grounded weights rather than machine-generated
//! benchmark numbers. Each dispatchable's weight is derived from the storage reads/writes
//! it actually performs (via `T::DbWeight`, i.e. `RocksDbWeight` in the runtime) plus a
//! generous fixed compute allowance. In particular, dispatchables that run on-chain
//! ML-DSA-87 (FIPS 204) verification (`validate_block` when the selected validator has a
//! registered key, and `verify_pq_signature`) include a ~300µs `ML_DSA_VERIFY` allowance.
//!
//! They deliberately OVER-estimate so block space cannot be exhausted by under-priced
//! calls. The `benchmarking` module (built under `--features runtime-benchmarks`) can be
//! used with `frame-benchmarking` to replace these with empirical, machine-specific
//! weights before mainnet; this impl provides the complete `WeightInfo` trait in the
//! meantime and is what the runtime binds.

use crate::{Config, WeightInfo};
use core::marker::PhantomData;
use frame_support::{traits::Get, weights::Weight};

/// Conservative allowance for one on-chain ML-DSA-87 (FIPS 204) signature verification.
/// ML-DSA-87 verification costs on the order of a few hundred microseconds; 300µs
/// (300_000_000 ps of ref-time) is a deliberate over-estimate.
const ML_DSA_VERIFY: Weight = Weight::from_parts(300_000_000, 0);

/// Weight functions for `pallet-ghost-consensus`, parameterised by the runtime `T` so the
/// real `DbWeight` and `MaxValidators` are used.
pub struct SubstrateWeight<T>(PhantomData<T>);

impl<T: Config> WeightInfo for SubstrateWeight<T> {
    /// Phase check + parent/difficulty reads + stake-weighted selection over up to
    /// `MaxValidators` stakers, then either entering PoS validation or finalizing
    /// immediately. Reads scale with the staker set; writes are bounded.
    fn submit_block() -> Weight {
        let max_validators = T::MaxValidators::get() as u64;
        T::DbWeight::get()
            .reads_writes(4u64.saturating_add(max_validators), 8)
            .saturating_add(Weight::from_parts(20_000_000, 0))
    }

    /// Balance transfer into the pallet account plus stake/count bookkeeping.
    fn stake() -> Weight {
        T::DbWeight::get()
            .reads_writes(6, 6)
            .saturating_add(Weight::from_parts(15_000_000, 0))
    }

    /// Balance transfer back to the staker plus stake/count bookkeeping.
    fn unstake() -> Weight {
        T::DbWeight::get()
            .reads_writes(6, 6)
            .saturating_add(Weight::from_parts(15_000_000, 0))
    }

    /// Worst case: the selected validator has a registered ML-DSA key, so this runs a
    /// real on-chain ML-DSA-87 verification, then distributes rewards to the miner and up
    /// to `MaxValidators` stakers.
    fn validate_block() -> Weight {
        let max_validators = T::MaxValidators::get() as u64;
        T::DbWeight::get()
            .reads_writes(
                8u64.saturating_add(max_validators),
                8u64.saturating_add(max_validators),
            )
            .saturating_add(ML_DSA_VERIFY)
            .saturating_add(Weight::from_parts(20_000_000, 0))
    }

    /// Evidence validation plus a slashing-record push and stake reduction.
    fn report_misbehavior() -> Weight {
        T::DbWeight::get()
            .reads_writes(6, 6)
            .saturating_add(Weight::from_parts(20_000_000, 0))
    }

    fn register_pq_readiness(metadata_len: u32) -> Weight {
        T::DbWeight::get()
            .reads_writes(3, 3)
            .saturating_add(Weight::from_parts(10_000_000, 0))
            .saturating_add(Weight::from_parts(2_000, 0).saturating_mul(metadata_len.into()))
    }

    fn attest_pq_readiness(proof_len: u32) -> Weight {
        T::DbWeight::get()
            .reads_writes(4, 4)
            .saturating_add(Weight::from_parts(15_000_000, 0))
            .saturating_add(Weight::from_parts(2_000, 0).saturating_mul(proof_len.into()))
    }

    fn remove_pq_readiness() -> Weight {
        T::DbWeight::get()
            .reads_writes(3, 3)
            .saturating_add(Weight::from_parts(10_000_000, 0))
    }

    /// Validates the ML-DSA public-key length/algorithm, then stores key + algorithm.
    fn register_ml_dsa_key(key_len: u32) -> Weight {
        T::DbWeight::get()
            .reads_writes(2, 2)
            .saturating_add(Weight::from_parts(15_000_000, 0))
            .saturating_add(Weight::from_parts(300, 0).saturating_mul(key_len.into()))
    }

    /// Dominated by the on-chain ML-DSA-87 verification; also reads the registered key and
    /// records the attestation (with the replay-guard existence check).
    fn verify_pq_signature(sig_len: u32) -> Weight {
        T::DbWeight::get()
            .reads_writes(3, 2)
            .saturating_add(ML_DSA_VERIFY)
            .saturating_add(Weight::from_parts(300, 0).saturating_mul(sig_len.into()))
    }
}
