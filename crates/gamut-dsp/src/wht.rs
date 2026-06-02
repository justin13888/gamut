//! AV1 lossless 4×4 Walsh–Hadamard transform (AV1 §7.13.2.10) and its exact integer inverse.
//!
//! In AV1, a block is coded losslessly when `base_q_idx == 0` and all delta-Q are zero
//! (`CodedLossless == 1`, AV1 §5.9.12). The transform is then forced to `TX_4X4` and uses the
//! Walsh–Hadamard transform rather than the DCT (AV1 §7.13.3: "If Lossless is equal to 1, invoke
//! the Inverse WHT process"). The decoder reconstruct for one 4×4 lossless block is:
//!
//! 1. Dequant: `Dequant = Quant * 4` (8-bit `dc_q(0) == ac_q(0) == 4`, AV1 §7.12.2/.3).
//! 2. Row pass: 1-D inverse WHT with `shift = 2` on each row — the `>> 2` exactly cancels the
//!    `* 4` dequant, recovering `Quant` before the butterfly (AV1 §7.13.3).
//! 3. Column pass: 1-D inverse WHT with `shift = 0` on each column → residual.
//!
//! [`iwht4x4`] reproduces that decoder reconstruct exactly (it is the oracle for the dav1d
//! cross-check). [`fwht4x4`] is the encoder forward transform: the algebraic inverse, so
//! `iwht4x4(fwht4x4(residual)) == residual` for every input. Because the `* 4` / `>> 2` cancel,
//! the encoder emits the forward-transform output as the coded coefficients with no extra scaling.

/// 1-D inverse Walsh–Hadamard transform, in place over `t` (AV1 §7.13.2.10).
///
/// `shift` is the per-element pre-scaling right shift applied before the butterfly.
fn iwht_1d(t: [i64; 4], shift: u32) -> [i64; 4] {
    let mut a = t[0] >> shift;
    let mut c = t[1] >> shift;
    let mut d = t[2] >> shift;
    let mut b = t[3] >> shift;
    a += c;
    d -= b;
    let e = (a - d) >> 1;
    b = e - b;
    c = e - c;
    a -= b;
    d += c;
    [a, b, c, d]
}

/// Exact inverse of the `shift = 0` [`iwht_1d`] butterfly. Recovers the butterfly's input from its
/// output; used by the encoder forward transform.
fn iwht_1d_inverse(o: [i64; 4]) -> [i64; 4] {
    // Forward (shift 0): a1=in0+in1; d1=in2-in3; e=(a1-d1)>>1; out1=e-in3; out2=e-in1;
    //                    out0=a1-out1; out3=d1+out2.
    let a1 = o[0] + o[1];
    let d1 = o[3] - o[2];
    let e = (a1 - d1) >> 1;
    let in3 = e - o[1];
    let in1 = e - o[2];
    let in0 = a1 - in1;
    let in2 = d1 + in3;
    [in0, in1, in2, in3]
}

/// Forward 4×4 Walsh–Hadamard transform: maps a row-major residual block to the lossless AV1
/// coefficients to be entropy-coded.
///
/// The decoder applies a row inverse-WHT then a column inverse-WHT; this inverts that pipeline by
/// applying the inverse butterfly to columns then rows. `iwht4x4(fwht4x4(r)) == r` for all `r`.
#[must_use]
pub fn fwht4x4(residual: &[i32; 16]) -> [i32; 16] {
    // Undo the decoder's column pass: inverse butterfly down each column.
    let mut m = [[0i64; 4]; 4];
    for j in 0..4 {
        let col = [
            i64::from(residual[j]),
            i64::from(residual[4 + j]),
            i64::from(residual[8 + j]),
            i64::from(residual[12 + j]),
        ];
        let r = iwht_1d_inverse(col);
        for i in 0..4 {
            m[i][j] = r[i];
        }
    }
    // Undo the decoder's row pass: inverse butterfly across each row.
    let mut quant = [0i32; 16];
    for i in 0..4 {
        let r = iwht_1d_inverse(m[i]);
        for j in 0..4 {
            quant[i * 4 + j] = r[j] as i32;
        }
    }
    quant
}

/// Lossless 4×4 reconstruct: dequantizes (`* 4`), applies the AV1 row (shift 2) and column
/// (shift 0) inverse WHT, and returns the residual. Bit-exact with the AV1 decoder for an 8-bit
/// `base_q_idx == 0` block (AV1 §7.12.3, §7.13.3, §7.13.2.10).
#[must_use]
pub fn iwht4x4(quant: &[i32; 16]) -> [i32; 16] {
    // Row pass with shift = 2 over Dequant = Quant * 4; the >> 2 cancels the * 4.
    let mut m = [[0i64; 4]; 4];
    for i in 0..4 {
        let t = [
            i64::from(quant[i * 4]) * 4,
            i64::from(quant[i * 4 + 1]) * 4,
            i64::from(quant[i * 4 + 2]) * 4,
            i64::from(quant[i * 4 + 3]) * 4,
        ];
        m[i] = iwht_1d(t, 2);
    }
    // Column pass with shift = 0.
    let mut residual = [0i32; 16];
    for j in 0..4 {
        let t = [m[0][j], m[1][j], m[2][j], m[3][j]];
        let r = iwht_1d(t, 0);
        for i in 0..4 {
            residual[i * 4 + j] = r[i] as i32;
        }
    }
    residual
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small deterministic LCG (reproducible, no `rand` dependency).
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0
        }
        /// Residual sample in [-255, 255].
        fn residual(&mut self) -> i32 {
            (self.next() >> 33) as i32 % 511 - 255
        }
    }

    #[test]
    fn roundtrip_zero() {
        let r = [0i32; 16];
        assert_eq!(iwht4x4(&fwht4x4(&r)), r);
    }

    #[test]
    fn roundtrip_extremes() {
        // Checkerboard ±255 maximises Walsh–Hadamard coefficient magnitude; verifies no overflow
        // and no decoder-side dequant clamp (Quant * 4 must stay within [-32768, 32767]).
        let mut r = [0i32; 16];
        for (idx, v) in r.iter_mut().enumerate() {
            let row = idx / 4;
            let col = idx % 4;
            *v = if (row + col) % 2 == 0 { 255 } else { -255 };
        }
        let q = fwht4x4(&r);
        for &c in &q {
            assert!(
                c * 4 >= -32768 && c * 4 <= 32767,
                "coeff {c} would clamp on dequant"
            );
        }
        assert_eq!(iwht4x4(&q), r);
    }

    #[test]
    fn roundtrip_random() {
        let mut rng = Lcg(0xabcd_1234_5678_9999);
        for _ in 0..20_000 {
            let mut r = [0i32; 16];
            for v in &mut r {
                *v = rng.residual();
            }
            assert_eq!(iwht4x4(&fwht4x4(&r)), r);
        }
    }

    #[test]
    fn dc_only_residual() {
        // A flat residual transforms to a single DC coefficient and back.
        let r = [7i32; 16];
        let q = fwht4x4(&r);
        assert_eq!(iwht4x4(&q), r);
    }
}
