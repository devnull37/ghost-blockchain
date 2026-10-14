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
/// (double BLAKE2-256) and interpreted as a big-endian U256 work factor; the
/// seal is valid when `value * difficulty <= U256::MAX`, i.e. when `value`
/// does not exceed `U256::MAX / difficulty`, where `difficulty` is the
/// per-block work factor returned by [`GhostPowApi::next_difficulty`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Encode, Decode, TypeInfo)]
pub struct GhostSeal {
    /// Counter the miner incremented until the PoW hash met the target.
    pub nonce: u64,
    /// Pre-hash of the header this seal was mined for (the header hash with
    /// the seal digest stripped). Embedded so fork choice can compare two
    /// seals deterministically (`break_tie` receives only seal bytes): on an
    /// equal-total-difficulty tie, the smaller `pre_hash` wins — the same
    /// ordering `HeaviestChain` applies, so import-time and selection-time
    /// fork choice cannot diverge. Verified against the real pre-hash on
    /// import, so it cannot be forged to steal a tie.
    pub pre_hash: [u8; 32],
}

/// `floor(value * num / den)` with saturating overflow semantics.
///
/// Dividing `value` by `den` *before* multiplying keeps the intermediate
/// terms inside `U256` whenever the true result fits: `value = q*den + r`
/// with `r < den`, so `r*num < u64 * u64` always fits and only `q*num` can
/// saturate — exactly the overflow the result itself cannot represent. This
/// is the single implementation both `pallet-ghost-consensus`'s retarget and
/// the node-side mirror use, so the two can never drift apart.
///
/// `den == 0` is unreachable from the retarget call sites but handled
/// defensively: it saturates to `U256::MAX` (unbounded quotient) rather than
/// panicking, so fuzz inputs cannot crash it.
pub fn mul_div_floor(value: U256, num: u64, den: u64) -> U256 {
    if num == 0 || value.is_zero() {
        return U256::zero();
    }
    if den == 0 {
        return U256::MAX;
    }
    let den_u = U256::from(den);
    let num_u = U256::from(num);
    let q = value / den_u;
    let r = value % den_u;
    q.saturating_mul(num_u).saturating_add(r * num_u / den_u)
}

/// Compute the next work-factor difficulty after a retarget boundary.
///
/// Single source of truth for `docs/ghost-consensus-design.md` §4:
/// `new = prev * clamp(expected_ms / elapsed_ms, 1/4, 4)`, floored at `min`.
/// `pallet-ghost-consensus::retarget` calls this with on-chain
/// `elapsed`/`expected` values and the node mirrors it in tooling — sharing
/// one function makes the two bit-identical for every input, including the
/// saturating `prev * expected` region where a naive multiply-then-divide
/// loses the quotient before it can clamp.
///
/// * `elapsed_ms == 0` (or more than 4x faster than target) is a max-up
///   retarget: `prev * 4` saturating.
/// * More than 4x slower than target is a max-down retarget: `floor(prev/4)`.
/// * `expected_ms == 0` is degenerate — the pallet only ever produces a
///   positive `expected` (`RetargetInterval * TargetBlockTimeMs`), so the
///   guard is defensive and returns `prev` unchanged.
/// * The result is floored at `min` (the genesis `MinDifficulty`, >= 1) so
///   difficulty can never reach the work factor 0 that would trivialise PoW.
pub fn compute_next_difficulty(prev: U256, elapsed_ms: u64, expected_ms: u64, min: U256) -> U256 {
    /// Per-retarget clamp on the work-factor multiplier, per the design doc:
    /// the new factor is pinned to `[prev/4, prev*4]`.
    const RETARGET_CLAMP: u64 = 4;

    if expected_ms == 0 {
        return prev;
    }
    let (num, den) = if elapsed_ms == 0 || elapsed_ms.saturating_mul(RETARGET_CLAMP) < expected_ms {
        // > 4x faster than target (or instant): clamp factor to 4.
        (RETARGET_CLAMP, 1u64)
    } else if elapsed_ms > expected_ms.saturating_mul(RETARGET_CLAMP) {
        // > 4x slower than target: clamp factor to 1/4.
        (1u64, RETARGET_CLAMP)
    } else {
        (expected_ms, elapsed_ms)
    };
    mul_div_floor(prev, num, den).max(min)
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

#[cfg(test)]
mod tests {
    use super::*;
    use codec::DecodeAll;
    use proptest::prelude::*;

    fn u256(v: [u8; 32]) -> U256 {
        U256::from_big_endian(&v)
    }

    #[test]
    fn seal_encode_is_u64le_plus_hash() {
        let seal = GhostSeal {
            nonce: 0xdead_beef_cafe_f00d,
            pre_hash: [0xabu8; 32],
        };
        let mut expected = 0xdead_beef_cafe_f00du64.to_le_bytes().to_vec();
        expected.extend_from_slice(&[0xabu8; 32]);
        assert_eq!(seal.encode(), expected);
        assert_eq!(seal.encode().len(), 40);
    }

    #[test]
    fn mul_div_floor_edge_cases() {
        // num == 0 or value == 0 collapse to zero.
        assert_eq!(mul_div_floor(U256::from(5u64), 0, 7), U256::zero());
        assert_eq!(mul_div_floor(U256::zero(), 5, 7), U256::zero());
        // den == 0 saturates to MAX rather than dividing by zero.
        assert_eq!(mul_div_floor(U256::from(5u64), 3, 0), U256::MAX);
        // Exactness in the common path.
        assert_eq!(
            mul_div_floor(U256::from(1_000u64), 3, 4),
            U256::from(750u64)
        );
        // Saturating result.
        assert_eq!(mul_div_floor(U256::MAX, 4, 1), U256::MAX);
        // Exact even when value*num would overflow: MAX * 1 / 2 == MAX/2.
        assert_eq!(mul_div_floor(U256::MAX, 1, 2), U256::MAX / 2);
    }

    proptest! {
        #[test]
        fn seal_decode_roundtrip(
            nonce in any::<u64>(),
            pre_hash in any::<[u8; 32]>(),
            tail in proptest::collection::vec(any::<u8>(), 0..16),
        ) {
            let seal = GhostSeal { nonce, pre_hash };
            let bytes = seal.encode();
            // Strict decode consumes the whole encoding.
            prop_assert_eq!(
                GhostSeal::decode_all(&mut &bytes[..]).unwrap(),
                seal
            );
            // Appended bytes are rejected by decode_all (strict) but not by
            // decode — the seal parser's acceptance surface stays pinned.
            let mut padded = bytes.clone();
            padded.extend_from_slice(&tail);
            prop_assert_eq!(
                GhostSeal::decode(&mut &padded[..]).unwrap(),
                seal
            );
            prop_assert!(GhostSeal::decode_all(&mut &padded[..]).is_err() != tail.is_empty());
        }

        #[test]
        fn seal_decode_never_panics_on_garbage(
            bytes in proptest::collection::vec(any::<u8>(), 0..80),
        ) {
            // Any outcome is fine (Ok/Err); the contract being pinned is that
            // arbitrary input is rejected or round-trips, never crashes.
            if let Ok(seal) = GhostSeal::decode_all(&mut &bytes[..]) {
                prop_assert_eq!(seal.encode(), bytes);
            }
        }

        #[test]
        fn mul_div_floor_matches_full_precision(
            v in any::<[u8; 32]>(),
            num in any::<u64>(),
            den in 1u64..,
        ) {
            let v = u256(v);
            let got = mul_div_floor(v, num, den);
            // Reference: full-precision floor(v*num/den) via U512, saturated
            // at U256::MAX — validates the divide-first implementation against
            // the mathematically exact result rather than restating it.
            let exact = v.full_mul(U256::from(num)) / sp_core::U512::from(den);
            let expected = U256::try_from(exact).unwrap_or(U256::MAX);
            prop_assert_eq!(got, expected);
        }

        #[test]
        fn retarget_matches_reference(
            prev in any::<[u8; 32]>(),
            elapsed in any::<u64>(),
            expected in 1u64..,
            min in any::<u64>(),
        ) {
            // Reference formula straight from the design doc:
            //   new = prev * clamp(expected/elapsed, 1/4, 4)  (floored at min)
            // computed at unbounded precision via U512 so this test validates
            // the implementation rather than restating it.
            let prev = u256(prev);
            let min = U256::from(min);
            let got = compute_next_difficulty(prev, elapsed, expected, min);

            let (num, den): (u64, u64) = if elapsed == 0 || elapsed.saturating_mul(4) < expected {
                (4, 1)
            } else if elapsed > expected.saturating_mul(4) {
                (1, 4)
            } else {
                (expected, elapsed)
            };
            let exact = prev.full_mul(U256::from(num)) / sp_core::U512::from(den);
            let expected_result = U256::try_from(exact).unwrap_or(U256::MAX).max(min);
            prop_assert_eq!(got, expected_result);
        }
    }
}
