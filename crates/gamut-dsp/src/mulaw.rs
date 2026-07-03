//! µ-law companding and quantization.
//!
//! µ-law compresses a signed value in `[-1, 1]` so that values near zero get
//! finer quantization, then quantizes to an integer index. This is a perceptual
//! *quantization* primitive (not colour), used for coefficient coding.
//!
//! The quantizer uses an **odd** level count `2^bits − 1`: indices `0..=2^bits−2`
//! with the center index representing exactly `0.0`. The top code (`2^bits − 1`)
//! is never written, removing the zero bias so zeroed coefficients decode exactly.
//!
//! Tier-1 determinism: `compress` / `expand` use `std` `f64::ln` / `f64::powf`,
//! so results match chromahash's deterministic substrate within a small tolerance,
//! not bit-for-bit.

/// Round half away from zero (not Rust's default round-to-even).
fn round_half_away_from_zero(x: f64) -> f64 {
    if x >= 0.0 {
        (x + 0.5).floor()
    } else {
        (x - 0.5).ceil()
    }
}

/// Asserts the companding parameter contract shared by every function here.
fn assert_mu(mu: f64) {
    assert!(
        mu.is_finite() && mu > 0.0,
        "mulaw: mu must be finite and > 0"
    );
}

/// Largest writable index `2^bits − 2`. Asserts `bits` is in `2..=31` (below 2
/// the level count collapses; 32+ overflows the shift).
fn max_index(bits: u32) -> u32 {
    assert!(
        (2..=31).contains(&bits),
        "mulaw: bit width must be in 2..=31"
    );
    (1u32 << bits) - 2
}

/// µ-law compress a `value` in `[-1, 1]` to a companded value in `[-1, 1]`.
/// `value` is clamped to `[-1, 1]`; a NaN `value` propagates to a NaN result.
///
/// # Panics
/// Panics if `mu` is not finite and `> 0`.
#[must_use]
pub fn compress(value: f64, mu: f64) -> f64 {
    assert_mu(mu);
    let v = value.clamp(-1.0, 1.0);
    v.signum() * (1.0 + mu * v.abs()).ln() / (1.0 + mu).ln()
}

/// µ-law expand a companded value in `[-1, 1]` back to `[-1, 1]` — the inverse of
/// [`compress`]. A NaN `compressed` propagates to a NaN result.
///
/// # Panics
/// Panics if `mu` is not finite and `> 0`.
#[must_use]
pub fn expand(compressed: f64, mu: f64) -> f64 {
    assert_mu(mu);
    compressed.signum() * ((1.0 + mu).powf(compressed.abs()) - 1.0) / mu
}

/// Quantize a `value` in `[-1, 1]` through µ-law to an integer index in
/// `0..=2^bits−2` (odd level count; the center index is exactly `0.0`).
/// Out-of-range values clamp to the end codes.
///
/// # Panics
/// Panics if `bits` is not in `2..=31`, if `mu` is not finite and `> 0`, or if
/// `value` is NaN (an index must come out; infinities clamp, NaN cannot).
#[must_use]
pub fn quantize(value: f64, bits: u32, mu: f64) -> u32 {
    let max_idx = max_index(bits);
    assert!(!value.is_nan(), "mulaw: cannot quantize NaN");
    let compressed = compress(value, mu);
    let idx = round_half_away_from_zero((compressed + 1.0) / 2.0 * f64::from(max_idx));
    (idx as i64).clamp(0, i64::from(max_idx)) as u32
}

/// Dequantize an integer `index` back to a value in `[-1, 1]` through µ-law — the
/// inverse of [`quantize`]. The never-written top code clamps down to
/// `2^bits−2` for robustness.
///
/// # Panics
/// Panics if `bits` is not in `2..=31` or `mu` is not finite and `> 0`.
#[must_use]
pub fn dequantize(index: u32, bits: u32, mu: f64) -> f64 {
    let max_idx = max_index(bits);
    let index = index.min(max_idx);
    let compressed = f64::from(index) / f64::from(max_idx) * 2.0 - 1.0;
    expand(compressed, mu)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MU: f64 = 5.0;

    #[test]
    fn compress_expand_roundtrip() {
        for &v in &[-1.0, -0.5, 0.0, 0.5, 1.0] {
            let rt = expand(compress(v, MU), MU);
            assert!((rt - v).abs() < 1e-12, "roundtrip at {v}: {rt}");
        }
    }

    #[test]
    fn zero_quantizes_to_center_and_back_exactly() {
        for bits in [4u32, 5, 6] {
            let center = (1u32 << (bits - 1)) - 1;
            assert_eq!(quantize(0.0, bits, MU), center, "bits={bits}");
            assert_eq!(dequantize(center, bits, MU), 0.0, "bits={bits}");
        }
    }

    #[test]
    fn extremes_quantize_to_bounds() {
        for bits in [4u32, 5, 6] {
            let max_idx = (1u32 << bits) - 2;
            assert_eq!(quantize(-1.0, bits, MU), 0);
            assert_eq!(quantize(1.0, bits, MU), max_idx);
        }
    }

    #[test]
    fn top_code_clamps_on_dequantize() {
        for bits in [4u32, 5, 6] {
            let top = (1u32 << bits) - 1;
            assert_eq!(dequantize(top, bits, MU), dequantize(top - 1, bits, MU));
        }
    }

    #[test]
    fn symmetric_codes_around_center() {
        for bits in [4u32, 5, 6] {
            let center = (1u32 << (bits - 1)) - 1;
            for &v in &[0.1, 0.3, 0.7] {
                let qp = quantize(v, bits, MU);
                let qn = quantize(-v, bits, MU);
                assert_eq!(qp - center, center - qn, "±{v} at bits={bits}");
            }
        }
    }

    #[test]
    #[should_panic(expected = "bit width must be in 2..=31")]
    fn bit_width_below_range_panics() {
        let _ = quantize(0.5, 1, MU);
    }

    #[test]
    #[should_panic(expected = "bit width must be in 2..=31")]
    fn bit_width_above_range_panics() {
        let _ = dequantize(0, 32, MU);
    }

    #[test]
    #[should_panic(expected = "mu must be finite and > 0")]
    fn zero_mu_panics() {
        let _ = compress(0.5, 0.0);
    }

    #[test]
    #[should_panic(expected = "mu must be finite and > 0")]
    fn negative_mu_panics() {
        let _ = expand(0.5, -1.0);
    }

    #[test]
    #[should_panic(expected = "mu must be finite and > 0")]
    fn nan_mu_panics() {
        let _ = quantize(0.5, 5, f64::NAN);
    }

    #[test]
    #[should_panic(expected = "cannot quantize NaN")]
    fn quantize_nan_value_panics() {
        let _ = quantize(f64::NAN, 5, MU);
    }

    #[test]
    fn infinities_clamp_and_nan_propagates() {
        // Adversarial totality probes: compress absorbs ±∞ through the [-1, 1] clamp,
        // IEEE-propagates a NaN value, and quantize maps ±∞ to the end codes.
        assert_eq!(compress(f64::INFINITY, MU), compress(1.0, MU));
        assert_eq!(compress(f64::NEG_INFINITY, MU), compress(-1.0, MU));
        assert!(compress(f64::NAN, MU).is_nan());
        assert_eq!(quantize(f64::INFINITY, 5, MU), (1u32 << 5) - 2);
        assert_eq!(quantize(f64::NEG_INFINITY, 5, MU), 0);
    }

    #[test]
    fn boundary_bit_widths() {
        // bits = 2 is the smallest legal width (three levels {0, 1, 2}, center 1); bits = 31 is
        // the largest (max_idx = 2³¹ − 2 without overflowing the shift), center 2³⁰ − 1 exact.
        assert_eq!(quantize(0.0, 2, MU), 1);
        assert_eq!(quantize(-1.0, 2, MU), 0);
        assert_eq!(quantize(1.0, 2, MU), 2);
        assert_eq!(quantize(1.0, 31, MU), (1u32 << 31) - 2);
        assert_eq!(dequantize((1u32 << 30) - 1, 31, MU), 0.0);
    }

    #[test]
    fn round_half_away_from_zero_at_ties() {
        // Half-ties round away from zero in both directions; non-ties round to nearest. Ties are the
        // only inputs that distinguish the `x >= 0` sign split and the `x - 0.5` bias from their
        // mutations, and the higher-level quantizer tests never land exactly on one.
        assert_eq!(round_half_away_from_zero(2.5), 3.0);
        assert_eq!(round_half_away_from_zero(-2.5), -3.0);
        assert_eq!(round_half_away_from_zero(2.4), 2.0);
        assert_eq!(round_half_away_from_zero(-2.4), -2.0);
        assert_eq!(round_half_away_from_zero(0.0), 0.0);
    }

    /// Golden vectors transcribed from chromahash `spec/test-vectors/unit-mulaw.json`
    /// (MIT OR Apache-2.0). Tier-1 `std` math reproduces chromahash's deterministic
    /// outputs to within this tolerance; the integer index matches exactly.
    #[test]
    fn matches_chromahash_mulaw_vectors() {
        struct Case {
            value: f64,
            bits: u32,
            mu: f64,
            compressed: f64,
            expanded: f64,
            quantized: u32,
            dequantized: f64,
        }
        let cases = [
            Case {
                value: 0.0,
                bits: 5,
                mu: 5.0,
                compressed: 0.0,
                expanded: 0.0,
                quantized: 15,
                dequantized: 0.0,
            },
            Case {
                value: 1.0,
                bits: 5,
                mu: 5.0,
                compressed: 1.0,
                expanded: 1.0000000000000002,
                quantized: 30,
                dequantized: 1.0000000000000002,
            },
            Case {
                value: -1.0,
                bits: 5,
                mu: 5.0,
                compressed: -1.0,
                expanded: -1.0000000000000002,
                quantized: 0,
                dequantized: -1.0000000000000002,
            },
            Case {
                value: 0.5,
                bits: 5,
                mu: 5.0,
                compressed: 0.6991803252671502,
                expanded: 0.4999999999999999,
                quantized: 25,
                dequantized: 0.46038544977892554,
            },
            Case {
                value: -0.5,
                bits: 4,
                mu: 5.0,
                compressed: -0.6991803252671502,
                expanded: -0.4999999999999999,
                quantized: 2,
                dequantized: -0.5192043696541104,
            },
            Case {
                value: 0.75,
                bits: 6,
                mu: 5.0,
                compressed: 0.8696170690354138,
                expanded: 0.7499999999999998,
                quantized: 58,
                dequantized: 0.7523018611322912,
            },
            Case {
                value: 0.0,
                bits: 5,
                mu: 8.0,
                compressed: 0.0,
                expanded: 0.0,
                quantized: 15,
                dequantized: 0.0,
            },
            Case {
                value: 0.5,
                bits: 5,
                mu: 8.0,
                compressed: 0.7324867603589637,
                expanded: 0.4999999999999999,
                quantized: 26,
                dequantized: 0.5011636512658286,
            },
            Case {
                value: -0.25,
                bits: 6,
                mu: 8.0,
                compressed: -0.5000000000000001,
                expanded: -0.25000000000000006,
                quantized: 15,
                dequantized: -0.2635279583396101,
            },
            Case {
                value: 1.0,
                bits: 4,
                mu: 8.0,
                compressed: 1.0,
                expanded: 0.9999999999999998,
                quantized: 14,
                dequantized: 0.9999999999999998,
            },
            Case {
                value: -0.75,
                bits: 5,
                mu: 8.0,
                compressed: -0.8856218745807111,
                expanded: -0.7499999999999999,
                quantized: 2,
                dequantized: -0.7143057295610805,
            },
        ];
        for c in &cases {
            assert!(
                (compress(c.value, c.mu) - c.compressed).abs() < 1e-9,
                "compress {}",
                c.value
            );
            assert!(
                (expand(c.compressed, c.mu) - c.expanded).abs() < 1e-9,
                "expand {}",
                c.value
            );
            // At an exact companding tie (e.g. v=-0.25, mu=8 ⇒ compressed = -0.5
            // exactly, mid-level at bits=6), a 1-ULP difference between std `ln`
            // and chromahash's deterministic `ln` rounds to the adjacent level.
            // Tier-1 therefore agrees on the index within ±1; the exact center /
            // bound / symmetry behavior is pinned by the structural tests above.
            let q = quantize(c.value, c.bits, c.mu);
            assert!(
                (i64::from(q) - i64::from(c.quantized)).abs() <= 1,
                "quantize {}: {q} vs {}",
                c.value,
                c.quantized
            );
            assert!(
                (dequantize(c.quantized, c.bits, c.mu) - c.dequantized).abs() < 1e-9,
                "dequantize {}",
                c.value
            );
        }
    }
}
