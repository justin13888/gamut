//! Shared integer arithmetic primitives for the gamut codecs.
//!
//! These are the small, algorithm-agnostic scalar operations that the codec crates would
//! otherwise each re-implement. Keeping them here gives every consumer one definition to
//! optimize. The first three are the AV1 §4.7 rounding/clamp operations (`Round2`,
//! `Round2Signed`, `Clip3`); [`round_div_nearest`] is the encoder's round-to-nearest divide
//! shared by the AV1 and VP8 forward quantizers.

/// `Round2(x, n)` (AV1 §4.7): rounding right shift, `x` for `n == 0` (arithmetic `>>`, ties up).
#[must_use]
pub fn round2(x: i64, n: u32) -> i64 {
    if n == 0 { x } else { (x + (1 << (n - 1))) >> n }
}

/// `Round2Signed(x, n)` (AV1 §4.7): symmetric rounding right shift — rounds the magnitude and
/// keeps the sign (ties away from zero), unlike [`round2`] which ties toward `+∞`.
///
/// Requires `n >= 1` (the rounding bias is `1 << (n - 1)`).
#[must_use]
pub fn round2_signed(x: i32, n: u32) -> i32 {
    if x >= 0 {
        (x + (1 << (n - 1))) >> n
    } else {
        -((-x + (1 << (n - 1))) >> n)
    }
}

/// `Clip3(low, high, x)` (AV1 §4.7): clamp `x` to the inclusive range `[low, high]`.
#[must_use]
pub fn clip3(low: i64, high: i64, x: i64) -> i64 {
    x.clamp(low, high)
}

/// Rounds `num / den` to the nearest integer, ties away from zero — the encoder forward-quantize
/// step shared by AV1 and VP8 (`level ≈ coeff / q`). Requires `den > 0`.
#[must_use]
pub fn round_div_nearest(num: i32, den: i32) -> i32 {
    let half = den / 2;
    if num >= 0 {
        (num + half) / den
    } else {
        -((-num + half) / den)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round2_matches_definition() {
        assert_eq!(round2(7, 0), 7);
        assert_eq!(round2(7, 1), 4); // (7 + 1) >> 1
        assert_eq!(round2(-1, 1), 0); // (-1 + 1) >> 1, ties toward +inf
        assert_eq!(round2(-3, 1), -1); // (-3 + 1) >> 1 = -1
        assert_eq!(round2(8, 2), 2); // (8 + 2) >> 2 = 2
    }

    #[test]
    fn round2_signed_rounds_magnitude_keeping_sign() {
        // Mirror image about zero: ties go away from zero in both directions.
        assert_eq!(round2_signed(7, 1), 4); // (7 + 1) >> 1
        assert_eq!(round2_signed(-7, 1), -4); // -((7 + 1) >> 1)
        assert_eq!(round2_signed(5, 2), 1); // (5 + 2) >> 2
        assert_eq!(round2_signed(-5, 2), -1);
        assert_eq!(round2_signed(0, 4), 0);
        // Differs from round2 on negative ties: round2(-1, 1) == 0, round2_signed(-1, 1) == -1.
        assert_eq!(round2_signed(-1, 1), -1);
    }

    #[test]
    fn clip3_clamps_inclusive() {
        assert_eq!(clip3(0, 255, -5), 0);
        assert_eq!(clip3(0, 255, 0), 0);
        assert_eq!(clip3(0, 255, 128), 128);
        assert_eq!(clip3(0, 255, 255), 255);
        assert_eq!(clip3(0, 255, 1000), 255);
        assert_eq!(clip3(-128, 127, -200), -128);
    }

    #[test]
    fn round_div_nearest_ties_away_from_zero() {
        assert_eq!(round_div_nearest(10, 4), 3); // 2.5 -> 3
        assert_eq!(round_div_nearest(-10, 4), -3); // -2.5 -> -3
        assert_eq!(round_div_nearest(9, 4), 2); // 2.25 -> 2
        assert_eq!(round_div_nearest(-9, 4), -2);
        assert_eq!(round_div_nearest(0, 7), 0);
        assert_eq!(round_div_nearest(7, 7), 1);
        // Sign symmetry across a range.
        for num in -50..=50 {
            assert_eq!(round_div_nearest(-num, 6), -round_div_nearest(num, 6));
        }
    }
}
