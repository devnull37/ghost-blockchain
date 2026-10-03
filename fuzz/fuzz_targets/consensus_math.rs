#![no_main]

//! Fuzz the consensus math every node applies identically:
//! `mul_div_floor` (the retarget's exact floor division), the retarget
//! `compute_next_difficulty`, and the seal-validity bound `pow_meets`.
//! Each is checked against a U512 full-precision reference computed from the
//! spec formula, so a saturating/arithmetic drift in production fails loudly.

use arbitrary::Arbitrary;
use ghost_consensus::{compute_next_difficulty, pow_meets};
use ghost_pow_primitives::mul_div_floor;
use libfuzzer_sys::fuzz_target;
use sp_core::{U256, U512};

#[derive(Arbitrary, Debug)]
struct Input {
    prev: [u8; 32],
    elapsed_ms: u64,
    expected_ms: u64,
    min: [u8; 32],
    num: u64,
    den: u64,
    value: [u8; 32],
}

fn saturate(exact: U512) -> U256 {
    U256::try_from(exact).unwrap_or(U256::MAX)
}

fuzz_target!(|input: Input| {
    let prev = U256::from_big_endian(&input.prev);
    let min = U256::from_big_endian(&input.min);
    let value = U256::from_big_endian(&input.value);

    // mul_div_floor == floor(v*num/den) at full precision, saturated at MAX.
    // den == 0 is the defensive saturate-to-MAX path.
    let got = mul_div_floor(value, input.num, input.den);
    if input.den == 0 || input.num == 0 || value.is_zero() {
        let expected = if input.den == 0 && input.num != 0 && !value.is_zero() {
            U256::MAX
        } else {
            U256::zero()
        };
        assert_eq!(got, expected);
    } else {
        let exact = value.full_mul(U256::from(input.num)) / U512::from(input.den);
        assert_eq!(got, saturate(exact));
    }

    // Retarget: spec formula `prev * clamp(expected/elapsed, 1/4, 4)` floored
    // at min — evaluated at unbounded precision.
    let next = compute_next_difficulty(prev, input.elapsed_ms, input.expected_ms, min);
    if input.expected_ms == 0 {
        assert_eq!(next, prev);
    } else {
        let (num, den): (u64, u64) =
            if input.elapsed_ms == 0 || input.elapsed_ms.saturating_mul(4) < input.expected_ms {
                (4, 1)
            } else if input.elapsed_ms > input.expected_ms.saturating_mul(4) {
                (1, 4)
            } else {
                (input.expected_ms, input.elapsed_ms)
            };
        let exact = prev.full_mul(U256::from(num)) / U512::from(den);
        assert_eq!(next, saturate(exact).max(min));
    }
    // Structural invariants regardless of the branch taken.
    if input.expected_ms != 0 {
        // The result is always floored at min, and the 4x down-clamp means it
        // can never drop below floor(prev/4) either.
        assert!(next >= min.max(prev / 4));
        if input.elapsed_ms == 0 || input.elapsed_ms.saturating_mul(4) < input.expected_ms {
            // Max-up retarget: exactly 4*prev (saturating), floored at min.
            assert_eq!(next, prev.saturating_mul(U256::from(4u64)).max(min));
        }
    }

    // pow_meets == `value * difficulty <= U256::MAX` evaluated at U512 —
    // except difficulty 0, which deliberately never meets: a zero work
    // factor would accept every seal and break heaviest-chain accounting.
    let meets = pow_meets(value, min);
    let exact = value.full_mul(min);
    assert_eq!(meets, !min.is_zero() && exact <= U512::from(U256::MAX));
    // Monotonicity: a harder work factor can never make a failing seal pass.
    let harder = min.saturating_add(U256::one());
    if pow_meets(value, harder) {
        assert!(meets || min.is_zero());
    }
});
