//! Difficulty retarget math (`docs/ghost-consensus-design.md` §4).
//!
//! `pallet-ghost-consensus` applies the retarget every `RETARGET_INTERVAL`
//! blocks in `on_initialize`. The formula lives in
//! `ghost_pow_primitives::compute_next_difficulty` — the single
//! implementation shared by the pallet and this node-side mirror — so the
//! two can never drift apart (a previous copy here computed
//! `prev * expected` before dividing and silently diverged from the pallet's
//! exact `mul_div_floor` for `prev > U256::MAX / expected`).
//!
//! `Difficulty` is a **work factor** (larger = harder), so the retarget moves
//! inversely to block speed: faster-than-target blocks (`elapsed < expected`)
//! *increase* the work factor; slower blocks decrease it.

pub use ghost_pow_primitives::compute_next_difficulty;

/// Retarget cadence in blocks (mirror of the pallet constant).
pub const RETARGET_INTERVAL: u32 = 100;
/// Target block time in milliseconds (mirror of the pallet constant).
pub const TARGET_BLOCK_TIME_MS: u64 = 5_000;

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use sp_core::U256;

    fn min() -> U256 {
        U256::one()
    }

    #[test]
    fn retarget_direction_and_clamps() {
        let prev = U256::from(1_000_000u64);
        let expected = u64::from(RETARGET_INTERVAL) * TARGET_BLOCK_TIME_MS;

        // Exactly on target: difficulty unchanged.
        assert_eq!(
            compute_next_difficulty(prev, expected, expected, min()),
            prev
        );

        // Blocks twice as fast as target: work factor doubles (harder).
        assert_eq!(
            compute_next_difficulty(prev, expected / 2, expected, min()),
            prev * 2
        );

        // Blocks twice as slow: work factor halves (easier).
        assert_eq!(
            compute_next_difficulty(prev, expected * 2, expected, min()),
            prev / 2
        );

        // Instant blocks: clamped to prev*4 (max-up), not higher.
        assert_eq!(compute_next_difficulty(prev, 0, expected, min()), prev * 4);

        // Absurdly slow: clamped to prev/4, not lower.
        assert_eq!(
            compute_next_difficulty(prev, u64::MAX, expected, min()),
            prev / 4
        );
    }

    #[test]
    fn retarget_floor_and_degenerate() {
        let expected = u64::from(RETARGET_INTERVAL) * TARGET_BLOCK_TIME_MS;

        // prev < 4: prev/4 == 0 but the floor lifts the result to min.
        assert_eq!(
            compute_next_difficulty(U256::from(3u64), u64::MAX, expected, min()),
            min()
        );
        // prev == 0 (must not happen on a live chain): floor still applies.
        assert_eq!(
            compute_next_difficulty(U256::zero(), 12345, expected, min()),
            min()
        );
        // A configured min above 1 is honoured.
        let min_bigger = U256::from(1_000u64);
        assert_eq!(
            compute_next_difficulty(U256::from(10u64), u64::MAX, expected, min_bigger),
            min_bigger
        );
        // Degenerate expected_ms: no retarget, no panic.
        assert_eq!(
            compute_next_difficulty(U256::from(7u64), 100, 0, min()),
            U256::from(7u64)
        );
    }

    #[test]
    fn retarget_is_exact_for_huge_prev() {
        // Regression: the old mirror computed `prev * expected` with a
        // saturating multiply *before* dividing, so for `prev > MAX/expected`
        // the quotient collapsed to ~MAX/elapsed and then got clamped —
        // diverging from the pallet's exact mul_div_floor (which returns the
        // mathematically correct floor(prev*num/den), saturating only the
        // final result).
        let expected = u64::from(RETARGET_INTERVAL) * TARGET_BLOCK_TIME_MS;
        assert_eq!(
            compute_next_difficulty(U256::MAX, expected, expected, min()),
            U256::MAX
        );
        assert_eq!(
            compute_next_difficulty(U256::MAX, expected * 2, expected, min()),
            U256::MAX / 2
        );
        // Max-up region: clamps to prev*4, saturated at MAX.
        assert_eq!(
            compute_next_difficulty(U256::MAX, 0, expected, min()),
            U256::MAX
        );
    }

    /// Independent reimplementation of the retarget as `pallet-ghost-consensus`
    /// originally computed it (branch selection + its own `mul_div_floor`).
    /// The production code now shares `compute_next_difficulty` between pallet
    /// and node; this third copy exists only as a tripwire — if either side's
    /// arithmetic ever drifts, the differential proptest below fails.
    fn reference_pallet_retarget(prev: U256, elapsed_ms: u64, expected_ms: u64, min: U256) -> U256 {
        fn mul_div_floor(value: U256, num: u64, den: u64) -> U256 {
            if num == 0 || value.is_zero() {
                return U256::zero();
            }
            let den = U256::from(den);
            let num = U256::from(num);
            let q = value / den;
            let r = value % den;
            q.saturating_mul(num).saturating_add(r * num / den)
        }

        // Mirrors the pallet branch structure exactly; `expected_ms == 0` is
        // unreachable on-chain (constants are nonzero) so no early return.
        let (num, den) = if elapsed_ms == 0 || elapsed_ms.saturating_mul(4) < expected_ms {
            (4u64, 1u64)
        } else if elapsed_ms > expected_ms.saturating_mul(4) {
            (1u64, 4u64)
        } else {
            (expected_ms, elapsed_ms)
        };
        mul_div_floor(prev, num, den).max(min)
    }

    proptest! {
        #[test]
        fn retarget_stays_within_bounds(
            prev in any::<[u8; 32]>(),
            elapsed in any::<u64>(),
            expected in 1u64..,
        ) {
            let prev = U256::from_big_endian(&prev);
            let min = min();
            let next = compute_next_difficulty(prev, elapsed, expected, min);
            // Result is >= prev/4 (the floor can only lift it) and <= prev*4,
            // except the min floor may push a degenerate prev above prev*4.
            prop_assert!(next >= prev / 4 || next == min);
            prop_assert!(next <= prev.saturating_mul(U256::from(4u64)).max(min));
            prop_assert!(next >= min);
        }

        #[test]
        fn retarget_direction_property(
            prev in 1u64..u32::MAX as u64,
            ratio_ppm in 1u64..40_000_000u64,
        ) {
            // expected = 1_000_000 ms; elapsed = ratio_ppm → raw factor is
            // expected/elapsed. elapsed >= expected (slow window) must not make
            // the work factor grow; elapsed < expected must not shrink it.
            let prev = U256::from(prev);
            let expected = 1_000_000u64;
            let elapsed = ratio_ppm;
            let next = compute_next_difficulty(prev, elapsed, expected, min());
            if elapsed >= expected {
                prop_assert!(next <= prev, "slow window must not make it harder");
            } else {
                prop_assert!(next >= prev, "fast window must not make it easier");
            }
        }

        #[test]
        fn retarget_matches_pallet_math(
            prev in any::<[u8; 32]>(),
            elapsed in any::<u64>(),
            expected in 1u64..,
            min in any::<u64>(),
        ) {
            // Bit-identical to the pallet's original arithmetic for every
            // input, including the saturating region where the former
            // multiply-then-clamp mirror diverged.
            let prev = U256::from_big_endian(&prev);
            let min = U256::from(min);
            prop_assert_eq!(
                compute_next_difficulty(prev, elapsed, expected, min),
                reference_pallet_retarget(prev, elapsed, expected, min)
            );
        }
    }
}
