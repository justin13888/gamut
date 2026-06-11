//! VP8 4×4 integer transforms (RFC 6386 §14.2–§14.4): the inverse DCT and inverse Walsh-Hadamard
//! transform that reconstruct residue from dequantized coefficients, plus their forward counterparts
//! for the encoder.
//!
//! These are **VP8-specific** and differ from AV1's transforms in `gamut-dsp`, so they live in-crate
//! (no second consumer). The inverse transforms are bit-exact ports of the reference C in RFC 6386
//! §14.3 (`vp8_short_inv_walsh4x4_c`) and §14.4 (`short_idct4x4llm_c`) — a conforming decoder must
//! match their rounding exactly. The forward transforms are the matched libvpx pair
//! (`vp8_short_fdct4x4_c` / `vp8_short_walsh4x4_c`); the encoder is free in its choice of forward
//! transform because reconstruction always re-derives residue through the inverse, but using the
//! standard pair keeps coefficients well-scaled. All blocks are 4×4, stored row-major; intermediate
//! and output values are 16-bit as the spec mandates. Tracked in `../STATUS.md` section L.

/// `sqrt(2) * sin(pi/8)` in 16-bit fixed point (RFC 6386 §14.4).
const SINPI8_SQRT2: i32 = 35468;
/// `sqrt(2) * cos(pi/8) - 1` in 16-bit fixed point (RFC 6386 §14.4).
const COSPI8_SQRT2_MINUS1: i32 = 20091;

/// Inverse 4×4 DCT (RFC 6386 §14.4): maps the 16 dequantized coefficients to the 4×4 residue,
/// bit-exact with the reference `short_idct4x4llm_c`. A vertical (column) pass feeds a horizontal
/// (row) pass; intermediates are stored as 16-bit as the spec requires.
#[must_use]
pub fn idct4x4(coeffs: &[i16; 16]) -> [i16; 16] {
    let mut tmp = [0i16; 16];
    // Vertical pass over each column.
    for col in 0..4 {
        let i0 = i32::from(coeffs[col]);
        let i4 = i32::from(coeffs[col + 4]);
        let i8 = i32::from(coeffs[col + 8]);
        let i12 = i32::from(coeffs[col + 12]);

        let a1 = i0 + i8;
        let b1 = i0 - i8;
        let c1 = ((i4 * SINPI8_SQRT2) >> 16) - (i12 + ((i12 * COSPI8_SQRT2_MINUS1) >> 16));
        let d1 = (i4 + ((i4 * COSPI8_SQRT2_MINUS1) >> 16)) + ((i12 * SINPI8_SQRT2) >> 16);

        tmp[col] = (a1 + d1) as i16;
        tmp[col + 12] = (a1 - d1) as i16;
        tmp[col + 4] = (b1 + c1) as i16;
        tmp[col + 8] = (b1 - c1) as i16;
    }
    // Horizontal pass over each row, with the final `+4 >> 3` normalization.
    let mut out = [0i16; 16];
    for row in 0..4 {
        let base = row * 4;
        let i0 = i32::from(tmp[base]);
        let i1 = i32::from(tmp[base + 1]);
        let i2 = i32::from(tmp[base + 2]);
        let i3 = i32::from(tmp[base + 3]);

        let a1 = i0 + i2;
        let b1 = i0 - i2;
        let c1 = ((i1 * SINPI8_SQRT2) >> 16) - (i3 + ((i3 * COSPI8_SQRT2_MINUS1) >> 16));
        let d1 = (i1 + ((i1 * COSPI8_SQRT2_MINUS1) >> 16)) + ((i3 * SINPI8_SQRT2) >> 16);

        out[base] = ((a1 + d1 + 4) >> 3) as i16;
        out[base + 3] = ((a1 - d1 + 4) >> 3) as i16;
        out[base + 1] = ((b1 + c1 + 4) >> 3) as i16;
        out[base + 2] = ((b1 - c1 + 4) >> 3) as i16;
    }
    out
}

/// Inverse 4×4 Walsh-Hadamard transform for the Y2 (luma-DC) block (RFC 6386 §14.3): maps the 16
/// dequantized Y2 coefficients to the 16 luma-subblock DC values, bit-exact with the reference
/// `vp8_short_inv_walsh4x4_c`.
#[must_use]
pub fn iwht4x4(coeffs: &[i16; 16]) -> [i16; 16] {
    let mut tmp = [0i16; 16];
    for col in 0..4 {
        let i0 = i32::from(coeffs[col]);
        let i4 = i32::from(coeffs[col + 4]);
        let i8 = i32::from(coeffs[col + 8]);
        let i12 = i32::from(coeffs[col + 12]);

        let a1 = i0 + i12;
        let b1 = i4 + i8;
        let c1 = i4 - i8;
        let d1 = i0 - i12;

        tmp[col] = (a1 + b1) as i16;
        tmp[col + 4] = (c1 + d1) as i16;
        tmp[col + 8] = (a1 - b1) as i16;
        tmp[col + 12] = (d1 - c1) as i16;
    }
    let mut out = [0i16; 16];
    for row in 0..4 {
        let base = row * 4;
        let i0 = i32::from(tmp[base]);
        let i1 = i32::from(tmp[base + 1]);
        let i2 = i32::from(tmp[base + 2]);
        let i3 = i32::from(tmp[base + 3]);

        let a1 = i0 + i3;
        let b1 = i1 + i2;
        let c1 = i1 - i2;
        let d1 = i0 - i3;

        out[base] = ((a1 + b1 + 3) >> 3) as i16;
        out[base + 1] = ((c1 + d1 + 3) >> 3) as i16;
        out[base + 2] = ((a1 - b1 + 3) >> 3) as i16;
        out[base + 3] = ((d1 - c1 + 3) >> 3) as i16;
    }
    out
}

/// Forward 4×4 DCT (libvpx `vp8_short_fdct4x4_c`): maps a 4×4 residue to coefficients. A horizontal
/// (row) pass feeds a vertical (column) pass.
#[must_use]
pub fn fdct4x4(residue: &[i16; 16]) -> [i16; 16] {
    let mut tmp = [0i16; 16];
    for row in 0..4 {
        let base = row * 4;
        let i0 = i32::from(residue[base]);
        let i1 = i32::from(residue[base + 1]);
        let i2 = i32::from(residue[base + 2]);
        let i3 = i32::from(residue[base + 3]);

        let a1 = (i0 + i3) * 8;
        let b1 = (i1 + i2) * 8;
        let c1 = (i1 - i2) * 8;
        let d1 = (i0 - i3) * 8;

        tmp[base] = (a1 + b1) as i16;
        tmp[base + 2] = (a1 - b1) as i16;
        tmp[base + 1] = ((c1 * 2217 + d1 * 5352 + 14500) >> 12) as i16;
        tmp[base + 3] = ((d1 * 2217 - c1 * 5352 + 7500) >> 12) as i16;
    }
    let mut out = [0i16; 16];
    for col in 0..4 {
        let i0 = i32::from(tmp[col]);
        let i4 = i32::from(tmp[col + 4]);
        let i8 = i32::from(tmp[col + 8]);
        let i12 = i32::from(tmp[col + 12]);

        let a1 = i0 + i12;
        let b1 = i4 + i8;
        let c1 = i4 - i8;
        let d1 = i0 - i12;

        out[col] = ((a1 + b1 + 7) >> 4) as i16;
        out[col + 8] = ((a1 - b1 + 7) >> 4) as i16;
        out[col + 4] = (((c1 * 2217 + d1 * 5352 + 12000) >> 16) + i32::from(d1 != 0)) as i16;
        out[col + 12] = ((d1 * 2217 - c1 * 5352 + 51000) >> 16) as i16;
    }
    out
}

/// Forward 4×4 Walsh-Hadamard transform for the Y2 block (libvpx `vp8_short_walsh4x4_c`): maps the 16
/// luma-subblock DC values to the 16 Y2 coefficients.
#[must_use]
pub fn fwht4x4(dc_values: &[i16; 16]) -> [i16; 16] {
    let mut tmp = [0i16; 16];
    for row in 0..4 {
        let base = row * 4;
        let i0 = i32::from(dc_values[base]);
        let i1 = i32::from(dc_values[base + 1]);
        let i2 = i32::from(dc_values[base + 2]);
        let i3 = i32::from(dc_values[base + 3]);

        let a1 = (i0 + i2) * 4;
        let d1 = (i1 + i3) * 4;
        let c1 = (i1 - i3) * 4;
        let b1 = (i0 - i2) * 4;

        tmp[base] = (a1 + d1 + i32::from(a1 != 0)) as i16;
        tmp[base + 1] = (b1 + c1) as i16;
        tmp[base + 2] = (b1 - c1) as i16;
        tmp[base + 3] = (a1 - d1) as i16;
    }
    let mut out = [0i16; 16];
    for col in 0..4 {
        let i0 = i32::from(tmp[col]);
        let i4 = i32::from(tmp[col + 4]);
        let i8 = i32::from(tmp[col + 8]);
        let i12 = i32::from(tmp[col + 12]);

        let a1 = i0 + i8;
        let d1 = i4 + i12;
        let c1 = i4 - i12;
        let b1 = i0 - i8;

        let mut a2 = a1 + d1;
        let mut b2 = b1 + c1;
        let mut c2 = b1 - c1;
        let mut d2 = a1 - d1;
        a2 += i32::from(a2 < 0);
        b2 += i32::from(b2 < 0);
        c2 += i32::from(c2 < 0);
        d2 += i32::from(d2 < 0);

        out[col] = ((a2 + 3) >> 3) as i16;
        out[col + 4] = ((b2 + 3) >> 3) as i16;
        out[col + 8] = ((c2 + 3) >> 3) as i16;
        out[col + 12] = ((d2 + 3) >> 3) as i16;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct SplitMix64(u64);
    impl SplitMix64 {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }
        /// A residue value in `-255..=255`.
        fn residue(&mut self) -> i16 {
            (self.next() % 511) as i16 - 255
        }
    }

    fn dc_block(dc: i16) -> [i16; 16] {
        let mut b = [0i16; 16];
        b[0] = dc;
        b
    }

    #[test]
    fn idct_of_pure_dc_is_flat() {
        // Inverse DCT of a single DC coefficient X yields a flat block of (X + 4) >> 3 (hand-derived
        // from §14.4: both passes collapse to the DC term).
        for dc in [8i16, 64, 100, -64, -8, 2047] {
            let out = idct4x4(&dc_block(dc));
            let expected = ((i32::from(dc) + 4) >> 3) as i16;
            assert!(
                out.iter().all(|&v| v == expected),
                "dc={dc} expected {expected}, got {out:?}"
            );
        }
    }

    #[test]
    fn iwht_of_pure_dc_is_flat() {
        // Inverse WHT of a single DC coefficient X yields a flat block of (X + 3) >> 3 (§14.3, and the
        // `vp8_short_inv_walsh4x4_1_c` fast path).
        for dc in [8i16, 24, 64, -24, -8] {
            let out = iwht4x4(&dc_block(dc));
            let expected = ((i32::from(dc) + 3) >> 3) as i16;
            assert!(
                out.iter().all(|&v| v == expected),
                "dc={dc} expected {expected}"
            );
        }
    }

    #[test]
    fn iwht_known_vector() {
        // Hand-derived from §14.3 for input [4,4,0,...]: both transformed axes collapse to
        // [1,1,0,0] on every row.
        let mut input = [0i16; 16];
        input[0] = 4;
        input[1] = 4;
        let expected = [1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0];
        assert_eq!(iwht4x4(&input), expected);
    }

    #[test]
    fn fdct_idct_roundtrip_within_tolerance() {
        // The forward/inverse DCT pair recovers the residue up to the small error of the asymmetric
        // integer rounding constants, which is what near-lossless quantization would also incur.
        // Exactness is not expected; bit-exact behavior is gated at P7 vs libwebp.
        let mut rng = SplitMix64(0xabcd_1234);
        let mut max_err = 0i32;
        for _ in 0..200 {
            let mut r = [0i16; 16];
            for v in &mut r {
                *v = rng.residue();
            }
            let back = idct4x4(&fdct4x4(&r));
            for i in 0..16 {
                max_err = max_err.max((i32::from(back[i]) - i32::from(r[i])).abs());
            }
        }
        assert!(max_err <= 2, "fdct∘idct error {max_err} exceeds tolerance");
    }

    #[test]
    fn fwht_iwht_roundtrip_within_tolerance() {
        let mut rng = SplitMix64(0x5555_aaaa);
        let mut max_err = 0i32;
        for _ in 0..200 {
            let mut x = [0i16; 16];
            for v in &mut x {
                *v = rng.residue();
            }
            let back = iwht4x4(&fwht4x4(&x));
            for i in 0..16 {
                max_err = max_err.max((i32::from(back[i]) - i32::from(x[i])).abs());
            }
        }
        assert!(max_err <= 2, "fwht∘iwht error {max_err} exceeds tolerance");
    }

    #[test]
    fn transforms_are_deterministic() {
        let mut b = [0i16; 16];
        for (i, v) in b.iter_mut().enumerate() {
            *v = (i as i16) * 3 - 20;
        }
        assert_eq!(idct4x4(&b), idct4x4(&b));
        assert_eq!(fdct4x4(&b), fdct4x4(&b));
        assert_eq!(iwht4x4(&b), iwht4x4(&b));
        assert_eq!(fwht4x4(&b), fwht4x4(&b));
    }
}
