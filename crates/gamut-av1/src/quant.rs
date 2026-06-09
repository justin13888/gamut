//! AV1 quantization: the `dc_q`/`ac_q` lookup tables and dequantization (AV1 §7.12.2/.3), plus a
//! straightforward encoder quantizer.
//!
//! [`dequant`] is the normative per-coefficient dequantization (the decoder operation); it must be
//! bit-exact and is pinned against the spec formula. [`quantize`] is the encoder operation —
//! choosing a coded level for a forward-transform coefficient. AV1 does not specify the encoder
//! quantizer, so this uses simple round-to-nearest; dead-zone biasing and rate-distortion-optimized
//! quantization (RDOQ) are quality tuning deferred to the encoder's decision layer.
//!
//! `base_q_idx == 0` gives `dc_q == ac_q == 4`, so for the lossless path [`dequant`] reduces to
//! `level * 4` — matching the `gamut_dsp` Walsh–Hadamard reconstruct.

mod tables;

use tables::{AC_QLOOKUP, DC_QLOOKUP};

/// Map a bit depth (8, 10, or 12) to the quantizer-table row index `(BitDepth - 8) >> 1`.
fn bit_depth_row(bit_depth: u32) -> usize {
    debug_assert!(
        matches!(bit_depth, 8 | 10 | 12),
        "bit_depth must be 8, 10, or 12"
    );
    ((bit_depth - 8) >> 1) as usize
}

/// `dc_q(b)` (AV1 §7.12.2): the DC quantizer step for quantizer index `b` at the given bit depth.
///
/// The index is clamped to `0..=255` as in the spec.
///
/// # Panics
/// Panics if `bit_depth` is not 8, 10, or 12.
#[must_use]
pub fn dc_q(bit_depth: u32, qindex: i32) -> i32 {
    DC_QLOOKUP[bit_depth_row(bit_depth)][qindex.clamp(0, 255) as usize]
}

/// `ac_q(b)` (AV1 §7.12.2): the AC quantizer step for quantizer index `b` at the given bit depth.
///
/// The index is clamped to `0..=255` as in the spec.
///
/// # Panics
/// Panics if `bit_depth` is not 8, 10, or 12.
#[must_use]
pub fn ac_q(bit_depth: u32, qindex: i32) -> i32 {
    AC_QLOOKUP[bit_depth_row(bit_depth)][qindex.clamp(0, 255) as usize]
}

/// Dequantize one coefficient (AV1 §7.12.3 steps b–f, without quantizer matrices).
///
/// `level` is the coded quantized coefficient, `q` the step from [`dc_q`]/[`ac_q`], `dq_denom` the
/// transform-size divisor (`1`, `2`, or `4`), and `bit_depth` selects the output clamp range. The
/// magnitude is masked to 24 bits and the result clamped to `±2^(7 + BitDepth)` exactly as the
/// decoder does, so this is bit-exact.
#[must_use]
pub fn dequant(level: i32, q: i32, dq_denom: i32, bit_depth: u32) -> i32 {
    let dq = i64::from(level) * i64::from(q);
    let sign = if dq < 0 { -1 } else { 1 };
    let dq2 = sign * ((dq.abs() & 0xFF_FFFF) / i64::from(dq_denom));
    let lim = 1i64 << (7 + bit_depth);
    dq2.clamp(-lim, lim - 1) as i32
}

/// Quantize one forward-transform coefficient to a coded level (encoder operation).
///
/// Round-to-nearest (ties away from zero): `level ≈ coeff / q`, so [`dequant`] of the result is the
/// representable value nearest `coeff`. This is a correct-but-untuned quantizer; dead-zone biasing
/// and RDOQ live in the encoder's rate-distortion layer.
///
/// # Panics
/// Panics if `q <= 0`.
#[must_use]
pub fn quantize(coeff: i32, q: i32) -> i32 {
    assert!(q > 0, "quantize: q must be positive");
    let half = q / 2;
    if coeff >= 0 {
        (coeff + half) / q
    } else {
        -((-coeff + half) / q)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0
        }
        fn range(&mut self, lo: i32, hi: i32) -> i32 {
            lo + (self.next() >> 33) as i32 % (hi - lo + 1)
        }
    }

    #[test]
    fn lookup_endpoints_match_spec() {
        // Spot values transcribed from AV1 §7.12.2.
        assert_eq!(dc_q(8, 0), 4);
        assert_eq!(dc_q(8, 255), 1336);
        assert_eq!(ac_q(8, 0), 4);
        assert_eq!(ac_q(8, 255), 1828);
        assert_eq!(dc_q(10, 255), 5347);
        assert_eq!(dc_q(12, 255), 21387);
        assert_eq!(ac_q(10, 255), 7312);
        assert_eq!(ac_q(12, 255), 29247);
    }

    #[test]
    fn lookup_clamps_index() {
        assert_eq!(dc_q(8, -5), dc_q(8, 0));
        assert_eq!(dc_q(8, 300), dc_q(8, 255));
        assert_eq!(ac_q(8, 1000), ac_q(8, 255));
    }

    #[test]
    fn lookup_tables_are_monotonic() {
        for &bd in &[8u32, 10, 12] {
            for q in 0..255 {
                assert!(dc_q(bd, q) <= dc_q(bd, q + 1), "dc_q bd={bd} q={q}");
                assert!(ac_q(bd, q) <= ac_q(bd, q + 1), "ac_q bd={bd} q={q}");
            }
        }
    }

    #[test]
    fn dequant_lossless_is_times_four() {
        // base_q_idx == 0 ⇒ q == 4, matching the Walsh–Hadamard `Dequant = Quant * 4`.
        assert_eq!(dc_q(8, 0), 4);
        for level in -100..=100 {
            assert_eq!(dequant(level, 4, 1, 8), level * 4);
        }
    }

    #[test]
    fn dequant_applies_denominator_and_clamp() {
        assert_eq!(dequant(100, 100, 4, 8), 2500); // (100*100)/4
        assert_eq!(dequant(100, 100, 2, 8), 5000);
        // Clamp to ±2^(7+8): (1<<15)-1 = 32767.
        assert_eq!(dequant(10_000, 1336, 1, 8), 32767);
        assert_eq!(dequant(-10_000, 1336, 1, 8), -32768);
    }

    #[test]
    fn quantize_then_dequant_is_within_one_step() {
        // The structural guarantee: the reconstruction nearest `coeff` is within one quant step.
        let mut rng = Lcg(0x9e37_79b9_7f4a_7c15);
        for _ in 0..20_000 {
            let qindex = rng.range(0, 255);
            let q = ac_q(8, qindex);
            // Keep level*q inside the 8-bit dequant clamp so the bound is about quantization only.
            let coeff = rng.range(-30_000, 30_000);
            let level = quantize(coeff, q);
            let recon = dequant(level, q, 1, 8);
            if recon.abs() < 32_767 {
                assert!(
                    (coeff - recon).abs() <= q,
                    "coeff={coeff} q={q} level={level} recon={recon}",
                );
            }
        }
    }

    #[test]
    fn quantize_is_sign_symmetric() {
        for &q in &[4, 17, 200, 1336] {
            for coeff in [-5000, -123, -1, 0, 1, 123, 5000] {
                assert_eq!(
                    quantize(coeff, q),
                    -quantize(-coeff, q),
                    "q={q} coeff={coeff}"
                );
            }
        }
    }
}
