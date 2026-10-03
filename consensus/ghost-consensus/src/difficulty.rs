//! Difficulty retarget math (`docs/ghost-consensus-design.md` §4).
//!
//! `pallet-ghost-consensus` applies this every `RETARGET_INTERVAL` blocks in
//! `on_initialize`; the node-side copy exists so the algorithm crate can
//! reason about the same rule in tests and tooling. The pallet's storage is
//! the source of truth at runtime — this function must stay bit-identical to
//! the pallet's math.
//!
//! `Difficulty` is a **work factor** (larger = harder), so the retarget moves
//! inversely to block speed: faster-than-target blocks (`elapsed < expected`)
//! *increase* the work factor; slower blocks decrease it.

use sp_core::U256;

/// Retarget cadence in blocks (mirror of the pallet constant).
pub const RETARGET_INTERVAL: u32 = 100;
/// Target block time in milliseconds (mirror of the pallet constant).
pub const TARGET_BLOCK_TIME_MS: u64 = 5_000;
/// Per-retarget clamp bounds: the new work factor is pinned to
/// `[prev/4, prev*4]`.
const RETARGET_CLAMP: u64 = 4;

/// Compute the next work-factor difficulty.
///
/// `next = prev * expected_ms / elapsed_ms`, where `elapsed_ms` is the actual
/// wall-clock time the last retarget window took and `expected_ms` is
/// `RETARGET_INTERVAL * TARGET_BLOCK_TIME_MS` — design doc §4:
/// `new_difficulty = old_difficulty * clamp(expected/elapsed, 0.25, 4)`.
///
/// The result is clamped to `[prev/4, prev*4]` and floored at `min` (the
/// genesis `MIN_DIFFICULTY`, ≥ 1). `elapsed_ms == 0` is treated as a max-up
/// retarget (`prev * 4`) per the doc; `expected_ms == 0` is degenerate and
/// returns `prev` unchanged — never a divide-by-zero or silent reset.
pub fn compute_next_difficulty(prev: U256, elapsed_ms: u64, expected_ms: u64, min: U256) -> U256 {
    if expected_ms == 0 {
        return prev;
    }
    let lower = prev / U256::from(RETARGET_CLAMP);
    let upper = prev.saturating_mul(U256::from(RETARGET_CLAMP));
    if elapsed_ms == 0 {
        return upper.max(min);
    }
    let scaled = prev
        .saturating_mul(U256::from(expected_ms))
        .checked_div(U256::from(elapsed_ms))
        .unwrap_or(prev);
    scaled.clamp(lower, upper).max(min)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

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
            prop_assert!(next <= prev.saturating_mul(U256::from(RETARGET_CLAMP)).max(min));
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
    }
}
