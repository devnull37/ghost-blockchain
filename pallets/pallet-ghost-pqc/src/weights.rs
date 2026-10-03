//! Weight constants for `pallet-ghost-pqc`.
//!
//! TODO(benchmarks): replace every constant with measured weights from
//! `frame-benchmarking` before any public-network claim. Signature verification
//! dominates: a single ML-DSA-87 `verify` is ~1-2M cycles native, and in the
//! Wasm runtime interpreter it is substantially slower — on the order of
//! several milliseconds of ref_time (hence the ~10ms placeholder below).
//! `register_pqc_key` and `pqc_attest` each perform exactly one verification.
//!
//! `proof_size` accounts for the ~7.2 KiB of key+signature data an extrinsic
//! carries plus the storage item.

use frame_support::weights::Weight;

/// Weight functions needed for `pallet-ghost-pqc`.
pub trait WeightInfo {
    fn register_pqc_key() -> Weight;
    fn revoke_pqc_key() -> Weight;
    fn pqc_attest() -> Weight;
    fn set_pqc_required() -> Weight;
}

/// Placeholder weights pending real benchmarks. Deliberately conservative.
///
/// NOTE: `Weight::from_parts(ref_time, proof_size)`; 1e9 ref_time ~= 1ms.
impl WeightInfo for () {
    /// One ML-DSA-87 verify + one storage write + ~7.2 KiB call data.
    fn register_pqc_key() -> Weight {
        Weight::from_parts(10_000_000_000, 8_000)
    }

    /// One storage read + one storage write.
    fn revoke_pqc_key() -> Weight {
        Weight::from_parts(30_000_000, 3_000)
    }

    /// One ML-DSA-87 verify + one storage read + ~4.6 KiB call data.
    fn pqc_attest() -> Weight {
        Weight::from_parts(10_000_000_000, 8_000)
    }

    /// One storage write.
    fn set_pqc_required() -> Weight {
        Weight::from_parts(10_000_000, 0)
    }
}
