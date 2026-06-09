//! Shared butterfly primitives for the AV1 1-D inverse transforms (AV1 §7.13.2.1).
//!
//! The inverse DCT and ADST processes are both expressed as sequences of two operations on a
//! working array `T`:
//!
//! - [`b`] — a butterfly *rotation* `B(a, b, angle, flip, r)` by `angle * pi / 128`, using the
//!   fixed-point cosine table [`COS128_LOOKUP`] (scale `4096`) and a `Round2(_, 12)` after each
//!   multiply-accumulate.
//! - [`h`] — a Hadamard *rotation* `H(a, b, flip, r)` (sum/difference) with `Clip3` to a signed
//!   `r`-bit range.
//!
//! These are a direct transcription of the spec and carry no scaling of their own; the per-pass
//! `Round2` shifts and the rectangular `sqrt(2)` scaling live in the 2-D process (AV1 §7.13.3).
//! `T` is held in `i64`: an 8-bit coefficient (≤ `2^16` after dequant) times `cos128` (≤ `2^12`)
//! stays far below the `i64` range, so the only saturation is the explicit `Clip3` inside [`h`].

/// `Cos128_Lookup` (AV1 §7.13.2.1): `round(4096 * cos(i * pi / 128))` for `i = 0..=64`.
pub(crate) const COS128_LOOKUP: [i64; 65] = [
    4096, 4095, 4091, 4085, 4076, 4065, 4052, 4036, 4017, 3996, 3973, 3948, 3920, 3889, 3857, 3822,
    3784, 3745, 3703, 3659, 3612, 3564, 3513, 3461, 3406, 3349, 3290, 3229, 3166, 3102, 3035, 2967,
    2896, 2824, 2751, 2675, 2598, 2520, 2440, 2359, 2276, 2191, 2106, 2019, 1931, 1842, 1751, 1660,
    1567, 1474, 1380, 1285, 1189, 1092, 995, 897, 799, 700, 601, 501, 401, 301, 201, 101, 0,
];

/// `cos128(angle)` (AV1 §7.13.2.1) — `4096 * cos(angle * pi / 128)`, exact, for any integer angle.
pub(crate) fn cos128(angle: i32) -> i64 {
    let angle2 = angle & 255;
    if angle2 <= 64 {
        COS128_LOOKUP[angle2 as usize]
    } else if angle2 <= 128 {
        -COS128_LOOKUP[(128 - angle2) as usize]
    } else if angle2 <= 192 {
        -COS128_LOOKUP[(angle2 - 128) as usize]
    } else {
        COS128_LOOKUP[(256 - angle2) as usize]
    }
}

/// `sin128(angle)` (AV1 §7.13.2.1), defined as `cos128(angle - 64)`.
pub(crate) fn sin128(angle: i32) -> i64 {
    cos128(angle - 64)
}

/// `Round2(x, n)` (AV1 §4.7): rounding right shift, `x` for `n == 0` (arithmetic `>>`, ties up).
pub(crate) fn round2(x: i64, n: u32) -> i64 {
    if n == 0 { x } else { (x + (1 << (n - 1))) >> n }
}

/// `Clip3(low, high, x)` (AV1 §4.7): clamp `x` to the inclusive range `[low, high]`.
pub(crate) fn clip3(low: i64, high: i64, x: i64) -> i64 {
    x.clamp(low, high)
}

/// `brev(num_bits, x)` (AV1 §7.13.2.1): reverse the low `num_bits` bits of `x`.
pub(crate) fn brev(num_bits: u32, x: i32) -> i32 {
    let mut t = 0;
    for i in 0..num_bits {
        let bit = (x >> i) & 1;
        t += bit << (num_bits - 1 - i);
    }
    t
}

/// `B(a, b, angle, flip, r)` (AV1 §7.13.2.1): butterfly rotation of `t[a]`, `t[b]` by `angle`.
///
/// The general multiply form is used for every angle; for the `32 + 64k` angles the spec's
/// reduced-multiplication form is defined to give identical results, so this is bit-exact.
pub(crate) fn b(t: &mut [i64], a: i32, bb: i32, angle: i32, flip: bool, _r: u32) {
    let (a, bb) = (a as usize, bb as usize);
    let cos = cos128(angle);
    let sin = sin128(angle);
    let x = t[a] * cos - t[bb] * sin;
    let y = t[a] * sin + t[bb] * cos;
    t[a] = round2(x, 12);
    t[bb] = round2(y, 12);
    if flip {
        t.swap(a, bb);
    }
}

/// `H(a, b, flip, r)` (AV1 §7.13.2.1): Hadamard rotation of `t[a]`, `t[b]`, clipped to `r` bits.
pub(crate) fn h(t: &mut [i64], a: i32, bb: i32, flip: bool, r: u32) {
    if flip {
        h(t, bb, a, false, r);
        return;
    }
    let (a, bb) = (a as usize, bb as usize);
    let lim = 1i64 << (r - 1);
    let x = t[a];
    let y = t[bb];
    t[a] = clip3(-lim, lim - 1, x + y);
    t[bb] = clip3(-lim, lim - 1, x - y);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cos128_matches_float_reference() {
        for angle in -300..=300 {
            let want = (4096.0 * (f64::from(angle) * std::f64::consts::PI / 128.0).cos()).round();
            assert_eq!(cos128(angle), want as i64, "cos128({angle})");
        }
    }

    #[test]
    fn sin128_is_cos_shifted() {
        for angle in -300..=300 {
            let want = (4096.0 * (f64::from(angle) * std::f64::consts::PI / 128.0).sin()).round();
            assert_eq!(sin128(angle), want as i64, "sin128({angle})");
        }
    }

    #[test]
    fn round2_matches_definition() {
        assert_eq!(round2(7, 0), 7);
        assert_eq!(round2(7, 1), 4); // (7 + 1) >> 1
        assert_eq!(round2(-1, 1), 0); // (-1 + 1) >> 1, ties toward +inf
        assert_eq!(round2(-3, 1), -1); // (-3 + 1) >> 1 = -1
        assert_eq!(round2(8, 2), 2); // (8 + 2) >> 2 = 2
    }

    #[test]
    fn brev_reverses_bits() {
        assert_eq!(brev(4, 0b0001), 0b1000);
        assert_eq!(brev(4, 0b0110), 0b0110);
        assert_eq!(brev(3, 0b001), 0b100);
        assert_eq!(brev(4, 0b1011), 0b1101);
    }

    #[test]
    fn hadamard_is_sum_and_difference() {
        let mut t = [5i64, 3];
        h(&mut t, 0, 1, false, 16);
        assert_eq!(t, [8, 2]);
        // Flip is H(b, a, 0): t[a] = t[b] - t[a], t[b] = t[b] + t[a].
        let mut t = [5i64, 3];
        h(&mut t, 0, 1, true, 16);
        assert_eq!(t, [-2, 8]);
    }
}
