//! CIE L\*a\*b\* colorimetry, ICC PCS fixed-point encodings, and colour-difference
//! metrics (ΔE\*ab, CIEDE2000).
//!
//! CIELab (CIE 15:2004) uses the **exact rational** junction constants ε = 216/24389 and
//! κ = 24389/27 ([`CIE_EPSILON`] / [`CIE_KAPPA`]) — never the rounded `0.008856` / `903.3`
//! forms: the two branches of the companding function are tangent only at the exact ε, so
//! the rounded pair opens a small discontinuity and shifts every dark colour (κ scales the
//! whole linear segment). CIE 15:2004 is paywalled, so the constants are transcribed in
//! `references/color/README.md`; the equations follow Bruce Lindbloom's transcription,
//! vendored as `references/color/lab-lindbloom.html`.
//!
//! The PCS codecs ([`encode_pcs_xyz`], [`encode_lab_v4_16`], [`encode_lab_v2_16`],
//! [`encode_lab_8`] and their decoders) are pure fixed-point number formats from
//! ICC.1:2022 Annex A and the legacy ICC.1:2001-04 §6.3.4 (vendored under
//! `references/icc/`). Their rounding (floor of `v + 0.5`, saturating) and their input
//! clamping replicate lcms2's `cmspcs.c` (`_cmsQuickSaturateWord`,
//! `cmsFloat2LabEncoded[V2]`, `cmsFloat2XYZEncoded`), so the planned lcms2 differential
//! tests (issue #322) can demand exact agreement.
//!
//! Colour difference: [`delta_e_76`] (CIE76, Euclidean) and [`delta_e_2000`] /
//! [`delta_e_2000_weighted`] (CIEDE2000), the latter implemented per Sharma, Wu & Dalal
//! (2005) — vendored as `references/color/ciede2000-sharma-2005.pdf` with the canonical
//! 34-pair golden set `ciede2000-testdata.txt` — including both hue-arithmetic traps their
//! "mathematical observations" flag: the ±360° adjustments when the hue angles straddle
//! 0°/360°, and the sum (not mean) h̄′ convention when `C1′·C2′ == 0`.
//!
//! **Deliberately out of scope:** ΔE94 and ΔE-CMC. Both are superseded by CIEDE2000 for
//! the workspace's use cases (conformance metrics, gamut statistics); omitting them is a
//! decision, not an oversight — they can be added later without reshaping this module.

/// CIE junction constant ε = 216/24389 (≈ 0.008856): the threshold between the cube-root
/// and linear branches of the Lab companding function. Exact rational per CIE 15:2004 /
/// ICC.1:2022 — deliberately not the rounded `0.008856`.
pub const CIE_EPSILON: f64 = 216.0 / 24389.0;

/// CIE junction constant κ = 24389/27 (≈ 903.3): the slope of the linear branch of the
/// Lab companding function. Exact rational per CIE 15:2004 / ICC.1:2022 — deliberately
/// not the rounded `903.3`. Note `κ·ε = 8` exactly (the L\* value at the junction).
pub const CIE_KAPPA: f64 = 24389.0 / 27.0;

/// The ICC PCS illuminant **D50** as `[X, Y, Z]`, at the exact u1Fixed15-derived rationals
/// the profile header mandates (ICC.1:2022 §7.2.16): `63190/65536`, `1`, `54061/65536` =
/// `(0.964202880859375, 1.0, 0.8249053955078125)` — matching
/// `gamut_icc::XyzNumber::D50.to_f64()` bit-for-bit.
///
/// lcms2 instead uses the rounded literals `0.9642` / `0.8249` (`cmsD50X/Y/Z` in
/// `lcms2.h`); the difference is ≈ 3e-6 per component and the planned lcms2 differential
/// tests (issue #322) absorb it at tolerance level.
pub const D50_XYZ: [f64; 3] = [63190.0 / 65536.0, 1.0, 54061.0 / 65536.0];

/// CIE standard illuminant **D65** as `[X, Y, Z]` at unit `Y`, derived from the CIE 1931
/// chromaticity `(0.3127, 0.3290)` ([`crate::matrix::D65`]) via the
/// [`xy_to_xyz`](crate::matrix::xy_to_xyz) construction: `X = x/y`, `Y = 1`,
/// `Z = (1 − x − y)/y`. The compiler folds the exact divisions, so this equals the runtime
/// derivation bit-for-bit (pinned by a test).
pub const D65_XYZ: [f64; 3] = [0.3127 / 0.3290, 1.0, (1.0 - 0.3127 - 0.3290) / 0.3290];

/// The Lab companding function `f(t)`: `t^(1/3)` for `t > ε`, else `(κ·t + 16)/116`.
fn lab_f(t: f64) -> f64 {
    if t > CIE_EPSILON {
        t.cbrt()
    } else {
        (CIE_KAPPA * t + 16.0) / 116.0
    }
}

/// Convert CIE XYZ to CIE L\*a\*b\* relative to the `white` tristimulus (e.g. [`D50_XYZ`]
/// for the ICC PCS, [`D65_XYZ`] for display work).
///
/// `L* = 116·f(Y/Yw) − 16`, `a* = 500·(f(X/Xw) − f(Y/Yw))`, `b* = 200·(f(Y/Yw) − f(Z/Zw))`
/// with the exact-rational `f` (see [`CIE_EPSILON`] / [`CIE_KAPPA`]). Negative tristimulus
/// components are handled by the linear branch (matching lcms2 `cmsXYZ2Lab`).
///
/// # Examples
///
/// ```
/// use gamut_color::lab::{D50_XYZ, xyz_to_lab};
/// // The white point maps to L* = 100 with zero chroma.
/// let lab = xyz_to_lab(D50_XYZ, D50_XYZ);
/// assert!((lab[0] - 100.0).abs() < 1e-12);
/// assert!(lab[1].abs() < 1e-12 && lab[2].abs() < 1e-12);
/// ```
#[must_use]
pub fn xyz_to_lab(xyz: [f64; 3], white: [f64; 3]) -> [f64; 3] {
    let fx = lab_f(xyz[0] / white[0]);
    let fy = lab_f(xyz[1] / white[1]);
    let fz = lab_f(xyz[2] / white[2]);
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// Convert CIE L\*a\*b\* to CIE XYZ relative to the `white` tristimulus.
///
/// Exact inverse of [`xyz_to_lab`] (branch-consistent per Lindbloom's `Lab_to_XYZ`, so
/// round trips are lossless to f64 rounding): `fy = (L+16)/116`, `fx = fy + a/500`,
/// `fz = fy − b/200`; each reduced tristimulus is `f⁻¹` applied with the matching
/// threshold (`fx³ > ε`, and `L > κ·ε = 8` for `Y`).
///
/// # Examples
///
/// ```
/// use gamut_color::lab::{D50_XYZ, lab_to_xyz};
/// let xyz = lab_to_xyz([100.0, 0.0, 0.0], D50_XYZ);
/// assert!((xyz[0] - D50_XYZ[0]).abs() < 1e-12);
/// assert!((xyz[1] - 1.0).abs() < 1e-12);
/// assert!((xyz[2] - D50_XYZ[2]).abs() < 1e-12);
/// ```
#[must_use]
pub fn lab_to_xyz(lab: [f64; 3], white: [f64; 3]) -> [f64; 3] {
    let fy = (lab[0] + 16.0) / 116.0;
    let fx = fy + lab[1] / 500.0;
    let fz = fy - lab[2] / 200.0;
    let xr = if fx * fx * fx > CIE_EPSILON {
        fx * fx * fx
    } else {
        (116.0 * fx - 16.0) / CIE_KAPPA
    };
    // κ·ε = 8 exactly, so `L > 8` is the Y-branch condition matching `f`'s `t > ε`.
    let yr = if lab[0] > CIE_KAPPA * CIE_EPSILON {
        fy * fy * fy
    } else {
        lab[0] / CIE_KAPPA
    };
    let zr = if fz * fz * fz > CIE_EPSILON {
        fz * fz * fz
    } else {
        (116.0 * fz - 16.0) / CIE_KAPPA
    };
    [xr * white[0], yr * white[1], zr * white[2]]
}

/// Convert CIE L\*a\*b\* to its cylindrical form CIE L\*C\*h(ab): `[L, C, h]` with
/// `C = √(a² + b²)` and `h = atan2(b, a)` in **degrees**, normalized to `[0, 360)`
/// (`h = 0` for the achromatic axis `a = b = 0`, matching `atan2(0, 0) = 0`).
///
/// # Examples
///
/// ```
/// use gamut_color::lab::lab_to_lch;
/// let lch = lab_to_lch([50.0, 0.0, -3.0]); // negative b* → h = 270°
/// assert!((lch[1] - 3.0).abs() < 1e-12);
/// assert!((lch[2] - 270.0).abs() < 1e-12);
/// ```
#[must_use]
pub fn lab_to_lch(lab: [f64; 3]) -> [f64; 3] {
    let c = lab[1].hypot(lab[2]);
    let h = lab[2].atan2(lab[1]).to_degrees();
    let h = if h < 0.0 { h + 360.0 } else { h };
    [lab[0], c, h]
}

/// Convert CIE L\*C\*h(ab) (`h` in degrees; any real angle is accepted — `cos`/`sin` wrap)
/// back to CIE L\*a\*b\*: `a = C·cos h`, `b = C·sin h`.
///
/// # Examples
///
/// ```
/// use gamut_color::lab::lch_to_lab;
/// let lab = lch_to_lab([50.0, 3.0, 270.0]);
/// assert!(lab[1].abs() < 1e-12);
/// assert!((lab[2] + 3.0).abs() < 1e-12);
/// ```
#[must_use]
pub fn lch_to_lab(lch: [f64; 3]) -> [f64; 3] {
    let h = lch[2].to_radians();
    [lch[0], lch[1] * h.cos(), lch[1] * h.sin()]
}

/// Convert CIE XYZ to CIE xyY: `x = X/(X+Y+Z)`, `y = Y/(X+Y+Z)`, luminance `Y` carried
/// through.
///
/// **Black-point convention:** when `X + Y + Z == 0` the chromaticity is undefined; this
/// returns `x = y = 0` (with the input `Y`), a fixed sentinel chosen over e.g. the
/// white-point chromaticity because this function takes no white argument. The inverse
/// [`xyy_to_xyz`] maps any `y == 0` input to black, so the round trip is closed.
#[must_use]
pub fn xyz_to_xyy(xyz: [f64; 3]) -> [f64; 3] {
    let sum = xyz[0] + xyz[1] + xyz[2];
    if sum == 0.0 {
        return [0.0, 0.0, xyz[1]];
    }
    [xyz[0] / sum, xyz[1] / sum, xyz[1]]
}

/// Convert CIE xyY to CIE XYZ: `X = x·Y/y`, `Z = (1 − x − y)·Y/y`.
///
/// **Black-point convention:** `y == 0` (the [`xyz_to_xyy`] black sentinel, or any point
/// on the alychne where luminance is zero) returns `[0, 0, 0]` rather than dividing by
/// zero.
///
/// # Examples
///
/// ```
/// use gamut_color::lab::{D65_XYZ, xyy_to_xyz, xyz_to_xyy};
/// let xyy = xyz_to_xyy(D65_XYZ);
/// assert!((xyy[0] - 0.3127).abs() < 1e-12 && (xyy[1] - 0.3290).abs() < 1e-12);
/// let xyz = xyy_to_xyz(xyy);
/// assert!((xyz[0] - D65_XYZ[0]).abs() < 1e-12);
/// ```
#[must_use]
pub fn xyy_to_xyz(xyy: [f64; 3]) -> [f64; 3] {
    if xyy[1] == 0.0 {
        return [0.0, 0.0, 0.0];
    }
    let scale = xyy[2] / xyy[1];
    [xyy[0] * scale, xyy[2], (1.0 - xyy[0] - xyy[1]) * scale]
}

/// lcms2 `_cmsQuickSaturateWord`: `floor(d + 0.5)` saturated to `0..=65535`. This is the
/// single rounding convention of every 16-bit PCS encoder here (round half up, not Rust's
/// round-half-away — identical on the non-negative post-clamp domain, but written as the
/// lcms2 expression so the issue #322 differential can demand exactness). lcms2's fast
/// floor quantizes to 1/65536 before flooring; that can differ from a true floor only when
/// `d + 0.5` lands within ~2⁻¹⁷ below an integer, which the clamped encoders never produce
/// from finite spec-range inputs.
fn quick_saturate_word(d: f64) -> u16 {
    let d = d + 0.5;
    if d <= 0.0 {
        return 0;
    }
    if d >= 65535.0 {
        return 0xFFFF;
    }
    d as u16 // truncation == floor: d is positive here
}

/// 8-bit analogue of [`quick_saturate_word`]: `floor(d + 0.5)` saturated to `0..=255`.
fn quick_saturate_byte(d: f64) -> u8 {
    let d = d + 0.5;
    if d <= 0.0 {
        return 0;
    }
    if d >= 255.0 {
        return 0xFF;
    }
    d as u8 // truncation == floor: d is positive here
}

/// Encode an XYZ tristimulus as PCSXYZ `u1Fixed15Number`s (ICC.1:2022 Annex A; legacy
/// ICC.1:2001-04 §6.3.4.1): `encoded = round(v · 32768)`, so the u16 range `0..=65535`
/// covers `0..=(1 + 32767/32768)`.
///
/// Clamping replicates lcms2 `cmsFloat2XYZEncoded`: if `Y <= 0` **all three** components
/// encode as 0 (a colour with no luminance is black); otherwise each component is clamped
/// to `[0, 65535/32768]` independently.
///
/// # Examples
///
/// ```
/// use gamut_color::lab::encode_pcs_xyz;
/// // 1.0 · 32768 = 0x8000; the top of the range is 65535/32768.
/// assert_eq!(encode_pcs_xyz([1.0, 1.0, 1.0]), [0x8000, 0x8000, 0x8000]);
/// assert_eq!(encode_pcs_xyz([2.0, 1.0, 0.0]), [0xFFFF, 0x8000, 0x0000]);
/// ```
#[must_use]
pub fn encode_pcs_xyz(xyz: [f64; 3]) -> [u16; 3] {
    const MAX_ENCODEABLE_XYZ: f64 = 1.0 + 32767.0 / 32768.0;
    if xyz[1] <= 0.0 {
        return [0, 0, 0];
    }
    let mut out = [0u16; 3];
    for (o, &v) in out.iter_mut().zip(xyz.iter()) {
        let v = v.clamp(0.0, MAX_ENCODEABLE_XYZ);
        *o = quick_saturate_word(v * 32768.0);
    }
    out
}

/// Decode PCSXYZ `u1Fixed15Number`s (see [`encode_pcs_xyz`]) to `f64`: `v / 32768`
/// (exact — lcms2 `cmsXYZEncoded2Float` shifts to s15Fixed16 and divides by 65536, the
/// same value).
#[must_use]
pub fn decode_pcs_xyz(xyz: [u16; 3]) -> [f64; 3] {
    [
        f64::from(xyz[0]) / 32768.0,
        f64::from(xyz[1]) / 32768.0,
        f64::from(xyz[2]) / 32768.0,
    ]
}

/// Encode CIELab as the ICC **v4** 16-bit PCSLAB encoding (ICC.1:2022 Annex A):
/// `L*: 0..100 → 0..0xFFFF` (`L · 65535/100 = L · 655.35`) and
/// `a*,b*: −128..+127 → 0..0xFFFF` (`(v + 128) · 65535/255 = (v + 128) · 257`).
///
/// Clamping and rounding replicate lcms2 `cmsFloat2LabEncoded`: `L` is clamped to
/// `[0, 100]`, `a`/`b` to `[−128, 127]`, then floor-of-`+0.5` rounded.
///
/// # Examples
///
/// ```
/// use gamut_color::lab::encode_lab_v4_16;
/// // L = 100 → 100·655.35 = 65535 = 0xFFFF; a = b = 0 → 128·257 = 32896 = 0x8080.
/// assert_eq!(encode_lab_v4_16([100.0, 0.0, 0.0]), [0xFFFF, 0x8080, 0x8080]);
/// ```
#[must_use]
pub fn encode_lab_v4_16(lab: [f64; 3]) -> [u16; 3] {
    let l = lab[0].clamp(0.0, 100.0);
    let a = lab[1].clamp(-128.0, 127.0);
    let b = lab[2].clamp(-128.0, 127.0);
    [
        quick_saturate_word(l * 655.35),
        quick_saturate_word((a + 128.0) * 257.0),
        quick_saturate_word((b + 128.0) * 257.0),
    ]
}

/// Decode the ICC **v4** 16-bit PCSLAB encoding (see [`encode_lab_v4_16`]):
/// `L = v/655.35`, `a,b = v/257 − 128` (lcms2 `cmsLabEncoded2Float`).
#[must_use]
pub fn decode_lab_v4_16(lab: [u16; 3]) -> [f64; 3] {
    [
        f64::from(lab[0]) / 655.35,
        f64::from(lab[1]) / 257.0 - 128.0,
        f64::from(lab[2]) / 257.0 - 128.0,
    ]
}

/// Encode CIELab as the **legacy v2** 16-bit PCSLAB encoding (ICC.1:2001-04 §6.3.4.2):
/// `L*: 0..100 → 0..0xFF00` (`L · 65280/100 = L · 652.8`) and
/// `a*,b*: −128..+127 → 0..0x8000..0xFF00` (`(v + 128) · 256`) — i.e. the v4 value scaled
/// by `65280/65535`. `0xFF00` is the nominal maximum; codes above it are representable
/// but out of nominal range, so out-of-range *input* clamps to the full-u16 top rather
/// than to the nominal top.
///
/// Clamping and rounding replicate lcms2 `cmsFloat2LabEncodedV2` exactly: `L` clamps to
/// `[0, 0xFFFF·100/0xFF00]` (= 100.390625), `a`/`b` to `[−128, 65535/256 − 128]`
/// (= 127.99609375), then floor-of-`+0.5` rounding.
///
/// # Examples
///
/// ```
/// use gamut_color::lab::encode_lab_v2_16;
/// // L = 100 → 100·652.8 = 65280 = 0xFF00 (not 0xFFFF — the v2/v4 scaling difference);
/// // a = b = 0 → 128·256 = 32768 = 0x8000.
/// assert_eq!(encode_lab_v2_16([100.0, 0.0, 0.0]), [0xFF00, 0x8000, 0x8000]);
/// ```
#[must_use]
pub fn encode_lab_v2_16(lab: [f64; 3]) -> [u16; 3] {
    // lcms2 Clamp_L_doubleV2 / Clamp_ab_doubleV2 bounds, written as lcms2 writes them.
    let l = lab[0].clamp(0.0, 65535.0 * 100.0 / 65280.0);
    let a = lab[1].clamp(-128.0, 65535.0 / 256.0 - 128.0);
    let b = lab[2].clamp(-128.0, 65535.0 / 256.0 - 128.0);
    [
        quick_saturate_word(l * 652.8),
        quick_saturate_word((a + 128.0) * 256.0),
        quick_saturate_word((b + 128.0) * 256.0),
    ]
}

/// Decode the **legacy v2** 16-bit PCSLAB encoding (see [`encode_lab_v2_16`]):
/// `L = v/652.8`, `a,b = v/256 − 128` (lcms2 `cmsLabEncoded2FloatV2`).
#[must_use]
pub fn decode_lab_v2_16(lab: [u16; 3]) -> [f64; 3] {
    [
        f64::from(lab[0]) / 652.8,
        f64::from(lab[1]) / 256.0 - 128.0,
        f64::from(lab[2]) / 256.0 - 128.0,
    ]
}

/// Encode CIELab as the 8-bit Lab encoding (ICC.1:2022 Annex A; same nominal ranges in
/// ICC.1:2001-04): `L*: 0..100 → 0..255` (`L · 255/100`) and `a*,b*: −128..+127 → 0..255`
/// (`v + 128` — the 8-bit a/b step is exactly 1). Inputs clamp to `[0, 100]` /
/// `[−128, 127]`, then floor-of-`+0.5` rounding.
///
/// Note: this is the spec-direct 8-bit mapping. lcms2 has no direct float→8-bit Lab
/// codec — its formatters widen 8-bit samples to 16-bit v2 by byte duplication — so the
/// issue #322 differential compares this pair against the spec, not against an lcms2
/// entry point.
///
/// # Examples
///
/// ```
/// use gamut_color::lab::encode_lab_8;
/// assert_eq!(encode_lab_8([100.0, 0.0, -128.0]), [255, 128, 0]);
/// ```
#[must_use]
pub fn encode_lab_8(lab: [f64; 3]) -> [u8; 3] {
    let l = lab[0].clamp(0.0, 100.0);
    let a = lab[1].clamp(-128.0, 127.0);
    let b = lab[2].clamp(-128.0, 127.0);
    [
        quick_saturate_byte(l * 255.0 / 100.0),
        quick_saturate_byte(a + 128.0),
        quick_saturate_byte(b + 128.0),
    ]
}

/// Decode the 8-bit Lab encoding (see [`encode_lab_8`]): `L = v·100/255`, `a,b = v − 128`.
#[must_use]
pub fn decode_lab_8(lab: [u8; 3]) -> [f64; 3] {
    [
        f64::from(lab[0]) * 100.0 / 255.0,
        f64::from(lab[1]) - 128.0,
        f64::from(lab[2]) - 128.0,
    ]
}

/// CIE76 colour difference ΔE\*ab: the Euclidean distance between two CIELab colours.
///
/// # Examples
///
/// ```
/// use gamut_color::delta_e_76;
/// // A 3-4-5 triangle in the (a, b) plane.
/// assert!((delta_e_76([50.0, 3.0, 4.0], [50.0, 0.0, 0.0]) - 5.0).abs() < 1e-12);
/// ```
#[must_use]
pub fn delta_e_76(lab1: [f64; 3], lab2: [f64; 3]) -> f64 {
    let dl = lab1[0] - lab2[0];
    let da = lab1[1] - lab2[1];
    let db = lab1[2] - lab2[2];
    (dl * dl + da * da + db * db).sqrt()
}

/// CIEDE2000 colour difference ΔE₀₀ with unit parametric weights
/// (`kL = kC = kH = 1`); see [`delta_e_2000_weighted`].
///
/// # Examples
///
/// ```
/// use gamut_color::delta_e_2000;
/// // First pair of the Sharma, Wu & Dalal (2005) golden set.
/// let de = delta_e_2000([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485]);
/// assert!((de - 2.0425).abs() < 1e-4);
/// ```
#[must_use]
pub fn delta_e_2000(lab1: [f64; 3], lab2: [f64; 3]) -> f64 {
    delta_e_2000_weighted(lab1, lab2, 1.0, 1.0, 1.0)
}

/// The modified hue angle `h′` of CIEDE2000 Eq. (7): `atan2(b, a′)` in degrees normalized
/// to `[0, 360)`, defined as 0 when `a′ = b = 0` (the paper's explicit convention, which
/// Eq. (14) exploits).
fn ciede2000_hue_deg(b: f64, a_prime: f64) -> f64 {
    if b == 0.0 && a_prime == 0.0 {
        return 0.0;
    }
    let h = b.atan2(a_prime).to_degrees();
    if h < 0.0 { h + 360.0 } else { h }
}

/// CIEDE2000 colour difference ΔE₀₀ with parametric weighting factors `kl` / `kc` / `kh`
/// (`kL`, `kC`, `kH` — all 1 for reference conditions).
///
/// Implemented exactly per Sharma, Wu & Dalal (2005), Eqs. (2)–(22)
/// (`references/color/ciede2000-sharma-2005.pdf`), including both traps from their
/// "mathematical observations": the ±360° hue-difference and mean-hue adjustments when
/// `|h1′ − h2′| > 180°` (Eqs. (10), (14)), and the `h̄′ = h1′ + h2′` **sum** convention
/// when `C1′·C2′ == 0` (Eq. (14) — the hue terms then vanish anyway since ΔH′ = 0).
/// Validated against the paper's canonical 34-pair golden set to 1e-4.
#[must_use]
pub fn delta_e_2000_weighted(lab1: [f64; 3], lab2: [f64; 3], kl: f64, kc: f64, kh: f64) -> f64 {
    /// 25⁷, the constant in the G (Eq. (4)) and R_C (Eq. (17)) chroma weightings.
    const POW7_25: f64 = 6_103_515_625.0;

    let (l1, a1, b1) = (lab1[0], lab1[1], lab1[2]);
    let (l2, a2, b2) = (lab2[0], lab2[1], lab2[2]);

    // Step 1: C′, h′ (Eqs. (2)–(7)).
    let c1_ab = a1.hypot(b1); // Eq. (2)
    let c2_ab = a2.hypot(b2);
    let c_bar_ab = 0.5 * (c1_ab + c2_ab); // Eq. (3)
    let c_bar_ab7 = c_bar_ab.powi(7);
    let g = 0.5 * (1.0 - (c_bar_ab7 / (c_bar_ab7 + POW7_25)).sqrt()); // Eq. (4)
    let a1p = (1.0 + g) * a1; // Eq. (5)
    let a2p = (1.0 + g) * a2;
    let c1p = a1p.hypot(b1); // Eq. (6)
    let c2p = a2p.hypot(b2);
    let h1p = ciede2000_hue_deg(b1, a1p); // Eq. (7)
    let h2p = ciede2000_hue_deg(b2, a2p);

    // Step 2: ΔL′, ΔC′, ΔH′ (Eqs. (8)–(11)).
    let dl = l2 - l1; // Eq. (8)
    let dc = c2p - c1p; // Eq. (9)
    // Eq. (10): hue difference, ±360°-adjusted into (−180°, 180°]; 0 if either chroma is 0.
    let dhp = if c1p * c2p == 0.0 {
        0.0
    } else {
        let d = h2p - h1p;
        if d > 180.0 {
            d - 360.0
        } else if d < -180.0 {
            d + 360.0
        } else {
            d
        }
    };
    let dh = 2.0 * (c1p * c2p).sqrt() * (0.5 * dhp).to_radians().sin(); // Eq. (11)

    // Step 3: the weighted combination (Eqs. (12)–(22)).
    let l_bar = 0.5 * (l1 + l2); // Eq. (12)
    let c_bar = 0.5 * (c1p + c2p); // Eq. (13)
    // Eq. (14): mean hue. When C1′·C2′ == 0 the paper defines h̄′ as the *sum* h1′ + h2′
    // (not the half-sum); otherwise the ±360° adjustment keeps the mean on the short arc.
    let h_bar = if c1p * c2p == 0.0 {
        h1p + h2p
    } else {
        let sum = h1p + h2p;
        if (h1p - h2p).abs() <= 180.0 {
            0.5 * sum
        } else if sum < 360.0 {
            0.5 * (sum + 360.0)
        } else {
            0.5 * (sum - 360.0)
        }
    };
    let t = 1.0 - 0.17 * (h_bar - 30.0).to_radians().cos()
        + 0.24 * (2.0 * h_bar).to_radians().cos()
        + 0.32 * (3.0 * h_bar + 6.0).to_radians().cos()
        - 0.20 * (4.0 * h_bar - 63.0).to_radians().cos(); // Eq. (15)
    let d_theta = 30.0 * (-((h_bar - 275.0) / 25.0).powi(2)).exp(); // Eq. (16)
    let c_bar7 = c_bar.powi(7);
    let rc = 2.0 * (c_bar7 / (c_bar7 + POW7_25)).sqrt(); // Eq. (17)
    let l50 = (l_bar - 50.0) * (l_bar - 50.0);
    let sl = 1.0 + 0.015 * l50 / (20.0 + l50).sqrt(); // Eq. (18)
    let sc = 1.0 + 0.045 * c_bar; // Eq. (19)
    let sh = 1.0 + 0.015 * c_bar * t; // Eq. (20)
    let rt = -(2.0 * d_theta).to_radians().sin() * rc; // Eq. (21)

    let tl = dl / (kl * sl);
    let tc = dc / (kc * sc);
    let th = dh / (kh * sh);
    (tl * tl + tc * tc + th * th + rt * tc * th).sqrt() // Eq. (22)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The canonical CIEDE2000 golden set from Sharma, Wu & Dalal (2005), Table 1:
    /// 34 tab-separated lines of `L1 a1 b1 L2 a2 b2 ΔE00`.
    const SHARMA_TESTDATA: &str = include_str!("../../../references/color/ciede2000-testdata.txt");

    /// The whole point of the ΔE₀₀ implementation: every pair of the Sharma golden set —
    /// designed specifically to exercise the hue-arithmetic branches (Eqs. (7), (10),
    /// (14)) that the CIE worked examples miss — must match to the paper's published
    /// 4-decimal precision, in both argument orders (the formula is symmetric, but a
    /// sign-convention mutation in Eq. (8)/(9)/(10) is not).
    #[test]
    fn ciede2000_matches_sharma_golden_set_both_orders() {
        let mut pairs = 0;
        for line in SHARMA_TESTDATA.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let v: Vec<f64> = line
                .split('\t')
                .map(|s| s.trim().parse().expect("numeric field"))
                .collect();
            assert_eq!(v.len(), 7, "expected 7 tab-separated fields, got {line:?}");
            let lab1 = [v[0], v[1], v[2]];
            let lab2 = [v[3], v[4], v[5]];
            let want = v[6];
            for (x, y) in [(lab1, lab2), (lab2, lab1)] {
                let got = delta_e_2000(x, y);
                assert!(
                    (got - want).abs() <= 1e-4,
                    "ΔE00({x:?}, {y:?}) = {got}, want {want}"
                );
            }
            pairs += 1;
        }
        assert_eq!(pairs, 34, "the Sharma golden set has 34 pairs");
    }

    /// ΔE₀₀ degenerate cases: identical colours are exactly 0, and an achromatic pair
    /// reduces to the pure lightness term ΔL′/S_L (every chroma/hue term vanishes).
    /// Catches mutations in the S_L weighting (Eq. (18)) that the golden set covers only
    /// mixed with chroma terms.
    #[test]
    fn ciede2000_degenerate_cases() {
        assert_eq!(delta_e_2000([53.1, 20.0, -34.5], [53.1, 20.0, -34.5]), 0.0);
        // Achromatic pair (50, 0, 0) vs (60, 0, 0): L̄′ = 55, S_L = 1 + 0.015·25/√45.
        let want = 10.0 / (1.0 + 0.015 * 25.0 / 45.0_f64.sqrt());
        let got = delta_e_2000([50.0, 0.0, 0.0], [60.0, 0.0, 0.0]);
        assert!((got - want).abs() < 1e-12, "got {got}, want {want}");
        // kL scales the lightness term through: kL = 2 halves the achromatic difference.
        let weighted = delta_e_2000_weighted([50.0, 0.0, 0.0], [60.0, 0.0, 0.0], 2.0, 1.0, 1.0);
        assert!((weighted - want / 2.0).abs() < 1e-12);
    }

    /// kC and kH must each divide their own term (a swapped or ignored weight would leave
    /// the golden set untouched, which only tests the k = 1 case). A pure-chroma pair
    /// isolates kC; a hue-differing pair must shrink as kH grows.
    #[test]
    fn ciede2000_parametric_weights_scale_their_terms() {
        // Same L and hue, different chroma: only the ΔC′/(kC·S_C) term is non-zero.
        let (p, q) = ([50.0, 10.0, 0.0], [50.0, 30.0, 0.0]);
        let unit = delta_e_2000(p, q);
        let kc2 = delta_e_2000_weighted(p, q, 1.0, 2.0, 1.0);
        assert!((kc2 - unit / 2.0).abs() < 1e-12, "kc2 {kc2} vs unit {unit}");
        // Hue-differing pair: raising kH must strictly reduce ΔE.
        let (p, q) = ([50.0, 20.0, 0.0], [50.0, 0.0, 20.0]);
        let unit = delta_e_2000(p, q);
        let kh2 = delta_e_2000_weighted(p, q, 1.0, 1.0, 2.0);
        assert!(kh2 < unit, "kh2 {kh2} should be < unit {unit}");
    }

    /// ΔE76 is the plain Euclidean metric — a hand-checkable 3-4-5-12-13 stack plus
    /// symmetry. A mutated component weight or dropped square root fails immediately.
    #[test]
    fn delta_e_76_is_euclidean() {
        assert_eq!(delta_e_76([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]), 0.0);
        let de = delta_e_76([12.0, 3.0, 4.0], [0.0, 0.0, 0.0]);
        assert!((de - 13.0).abs() < 1e-12);
        assert_eq!(
            delta_e_76([10.0, -5.0, 2.0], [1.0, 4.0, -3.0]),
            delta_e_76([1.0, 4.0, -3.0], [10.0, -5.0, 2.0])
        );
    }

    /// XYZ→Lab→XYZ across a grid of the PCS domain (0 ..= 65535/32768) under both
    /// standard whites: the branch conditions of `lab_to_xyz` mirror `f`'s exactly, so
    /// round trips must be lossless to f64 rounding — a mutation that mismatches the
    /// branch thresholds (e.g. `>=` for `>`, or a rounded ε) breaks the dark region.
    #[test]
    fn xyz_lab_round_trip_over_pcs_domain() {
        const MAX: f64 = 1.0 + 32767.0 / 32768.0;
        let steps: Vec<f64> = (0..=16).map(|i| f64::from(i) * MAX / 16.0).collect();
        for white in [D50_XYZ, D65_XYZ] {
            for &x in &steps {
                for &y in &steps {
                    for &z in &steps {
                        let xyz = [x, y, z];
                        let rt = lab_to_xyz(xyz_to_lab(xyz, white), white);
                        for i in 0..3 {
                            assert!(
                                (rt[i] - xyz[i]).abs() < 1e-12,
                                "round trip of {xyz:?} under {white:?}: {rt:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// Lab→XYZ→Lab over the full encodable Lab range (including out-of-gamut corners
    /// where intermediate tristimulus values go negative): the linear branch handles
    /// negative reduced tristimulus, so the round trip must still close.
    #[test]
    fn lab_xyz_round_trip_over_lab_range() {
        let ls = [0.0, 4.0, 8.0, 25.0, 50.0, 75.0, 100.0];
        let abs = [-128.0, -60.5, -1.0, 0.0, 0.25, 33.0, 127.0];
        for white in [D50_XYZ, D65_XYZ] {
            for &l in &ls {
                for &a in &abs {
                    for &b in &abs {
                        let lab = [l, a, b];
                        let rt = xyz_to_lab(lab_to_xyz(lab, white), white);
                        for i in 0..3 {
                            assert!(
                                (rt[i] - lab[i]).abs() < 1e-12,
                                "round trip of {lab:?} under {white:?}: {rt:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// The exact-rational requirement. At `t` between the *rounded* 0.008856 and the
    /// exact ε = 216/24389, the linear branch must apply, so `L == κ·t` to f64 rounding:
    /// substituting ε = 0.008856 flips this input to the cube-root branch (off by
    /// ≈ 2.15e-9 — the branches are tangent at the exact ε, so the error is second-order
    /// but still far above f64 noise), and substituting κ = 903.3 shifts L by ≈ 3.3e-5.
    /// At t = ε itself the two branches must agree: L = κ·ε = 8 exactly.
    #[test]
    fn lab_f_junction_requires_exact_rationals() {
        let t = 0.008_856_2; // 0.008856 < t < 216/24389
        let lab = xyz_to_lab([t, t, t], [1.0, 1.0, 1.0]);
        assert!(
            (lab[0] - CIE_KAPPA * t).abs() < 1e-10,
            "linear branch expected below ε: L = {}",
            lab[0]
        );
        // Junction continuity: both branches meet at L = κ·ε = 8 (216/24389 · 24389/27).
        let at = xyz_to_lab([CIE_EPSILON; 3], [1.0; 3]);
        let above = xyz_to_lab([CIE_EPSILON * (1.0 + 1e-13); 3], [1.0; 3]);
        assert!((at[0] - 8.0).abs() < 1e-9, "L at ε = {}", at[0]);
        assert!((above[0] - at[0]).abs() < 1e-9, "junction jump at ε");
    }

    /// D65_XYZ must equal the runtime `xy_to_xyz` derivation from the shared D65
    /// chromaticity bit-for-bit — const folding and runtime f64 use the same IEEE ops,
    /// so a transcription typo in either place breaks exact equality.
    #[test]
    fn d65_xyz_matches_xy_to_xyz_derivation() {
        let derived = crate::matrix::xy_to_xyz(crate::matrix::D65[0], crate::matrix::D65[1]);
        assert_eq!(D65_XYZ, derived);
    }

    /// PCSXYZ hand-derived vectors (ICC.1:2022 Annex A): 1.0 → 0x8000 (1·32768), the
    /// domain top 65535/32768 → 0xFFFF, and lcms2's `Y <= 0` rule zeroes all three
    /// components. Round trips stay within the u1Fixed15 quantization step.
    #[test]
    fn pcs_xyz_encoding_vectors_and_round_trip() {
        assert_eq!(encode_pcs_xyz([1.0, 1.0, 1.0]), [0x8000, 0x8000, 0x8000]);
        assert_eq!(encode_pcs_xyz([65535.0 / 32768.0; 3]), [0xFFFF; 3]);
        // Out-of-range clamps: above the top saturates, negative X/Z clamp to 0.
        assert_eq!(encode_pcs_xyz([3.0, 1.0, -0.5]), [0xFFFF, 0x8000, 0x0000]);
        // lcms2 cmsFloat2XYZEncoded: Y <= 0 encodes black regardless of X/Z.
        assert_eq!(encode_pcs_xyz([1.0, 0.0, 1.0]), [0, 0, 0]);
        assert_eq!(encode_pcs_xyz([1.0, -0.25, 1.0]), [0, 0, 0]);
        // decode: exact division by 32768.
        assert_eq!(decode_pcs_xyz([0x8000, 0, 0xFFFF])[0], 1.0);
        assert_eq!(decode_pcs_xyz([0x8000, 0, 0xFFFF])[2], 65535.0 / 32768.0);
        // encode∘decode round trip within half a quantization step.
        for v in [0.0, 1e-4, 0.18, 0.5, 1.0, 1.5, 65535.0 / 32768.0] {
            let dec = decode_pcs_xyz(encode_pcs_xyz([v; 3]));
            for d in dec {
                assert!((d - v).abs() <= 0.5 / 32768.0, "XYZ {v} decoded to {d}");
            }
        }
    }

    /// decode∘encode must be the identity on every u16 code (PCSXYZ sweep): the decoder
    /// and encoder use reciprocal scale factors, so any drift in either direction moves
    /// some code by one.
    #[test]
    fn pcs_xyz_decode_encode_identity_sweep() {
        for u in 0..=u16::MAX {
            // All three channels set to `u` so the `Y <= 0` rule only fires for u == 0.
            assert_eq!(encode_pcs_xyz(decode_pcs_xyz([u; 3])), [u; 3]);
        }
    }

    /// PCSLAB v4 hand-derived vectors (ICC.1:2022 Annex A). Derivations:
    /// L=100 → 100·655.35 = 65535 = 0xFFFF; a=b=0 → (0+128)·257 = 32896 = 0x8080;
    /// L=50 → 50·655.35 = 32767.5 exactly in f64 → floor(+0.5) = 32768 = 0x8000 (the
    /// round-half-up pin); a=−128 → 0; b=127 → 255·257 = 65535.
    #[test]
    fn pcs_lab_v4_encoding_vectors_and_round_trip() {
        assert_eq!(
            encode_lab_v4_16([100.0, 0.0, 0.0]),
            [0xFFFF, 0x8080, 0x8080]
        );
        assert_eq!(encode_lab_v4_16([50.0, -128.0, 127.0]), [0x8000, 0, 0xFFFF]);
        // Clamping: out-of-range input pins to the encoding extremes.
        assert_eq!(
            encode_lab_v4_16([120.0, -200.0, 300.0]),
            [0xFFFF, 0, 0xFFFF]
        );
        let dec = decode_lab_v4_16([0xFFFF, 0x8080, 0x8080]);
        assert!((dec[0] - 100.0).abs() < 1e-12);
        assert!(dec[1].abs() < 1e-12 && dec[2].abs() < 1e-12);
        // encode∘decode round trip within half a quantization step per channel.
        for lab in [[0.0, 0.0, 0.0], [42.17, -27.3, 88.8], [99.9, 126.9, -127.9]] {
            let dec = decode_lab_v4_16(encode_lab_v4_16(lab));
            assert!((dec[0] - lab[0]).abs() <= 0.5 / 655.35);
            assert!((dec[1] - lab[1]).abs() <= 0.5 / 257.0);
            assert!((dec[2] - lab[2]).abs() <= 0.5 / 257.0);
        }
    }

    /// PCSLAB v2 hand-derived vectors (ICC.1:2001-04 §6.3.4.2). Derivations:
    /// L=100 → 100·652.8 = 65280 = 0xFF00 (the nominal v2 top); a=b=0 → 128·256 = 0x8000;
    /// codes above 0xFF00 are legal but out of nominal range, so the lcms2 clamp tops at
    /// L = 100.390625 → 0xFFFF and a/b = 127.99609375 → 0xFFFF.
    #[test]
    fn pcs_lab_v2_encoding_vectors_and_round_trip() {
        assert_eq!(
            encode_lab_v2_16([100.0, 0.0, 0.0]),
            [0xFF00, 0x8000, 0x8000]
        );
        assert_eq!(encode_lab_v2_16([0.0, -128.0, 127.0]), [0, 0, 0xFF00]);
        // lcms2 V2 clamp: input above nominal range saturates the full u16, not 0xFF00.
        assert_eq!(encode_lab_v2_16([101.0, 128.0, 200.0]), [0xFFFF; 3]);
        let dec = decode_lab_v2_16([0xFF00, 0x8000, 0x8000]);
        assert!((dec[0] - 100.0).abs() < 1e-12);
        assert!(dec[1].abs() < 1e-12 && dec[2].abs() < 1e-12);
        for lab in [[0.0, 0.0, 0.0], [42.17, -27.3, 88.8], [99.9, 126.9, -127.9]] {
            let dec = decode_lab_v2_16(encode_lab_v2_16(lab));
            assert!((dec[0] - lab[0]).abs() <= 0.5 / 652.8);
            assert!((dec[1] - lab[1]).abs() <= 0.5 / 256.0);
            assert!((dec[2] - lab[2]).abs() <= 0.5 / 256.0);
        }
    }

    /// decode∘encode identity on every u16 L code for both 16-bit Lab encodings — the
    /// top-end sweep is where a v2/v4 scale mix-up (655.35 vs 652.8) or a rounding-mode
    /// drift shows up as an off-by-one.
    #[test]
    fn pcs_lab_16_decode_encode_identity_sweeps() {
        for u in 0..=u16::MAX {
            assert_eq!(encode_lab_v4_16(decode_lab_v4_16([u; 3])), [u; 3]);
            assert_eq!(encode_lab_v2_16(decode_lab_v2_16([u; 3])), [u; 3]);
        }
    }

    /// The v2/v4 top-end scaling difference: the two encodings scale by 65280 vs 65535,
    /// so they must disagree for any L high enough that the ≈0.39% scale gap exceeds one
    /// code (L ≥ ~0.4), and v2's L=100 code decoded *as v4* must read low by exactly
    /// 100·(1 − 65280/65535) ≈ 0.389. A "plausible-looking" wrong choice fails here.
    #[test]
    fn pcs_lab_v2_vs_v4_top_end_scaling_difference() {
        for l in [1.0, 25.0, 50.0, 75.0, 100.0] {
            let v2 = encode_lab_v2_16([l, 0.0, 0.0]);
            let v4 = encode_lab_v4_16([l, 0.0, 0.0]);
            assert_ne!(v2[0], v4[0], "v2 and v4 L codes must differ at L = {l}");
        }
        assert_eq!(encode_lab_v2_16([100.0, 0.0, 0.0])[0], 0xFF00);
        assert_eq!(encode_lab_v4_16([100.0, 0.0, 0.0])[0], 0xFFFF);
        let misread = decode_lab_v4_16([0xFF00, 0x8000, 0x8000])[0];
        let want = 100.0 * 65280.0 / 65535.0;
        assert!((misread - want).abs() < 1e-9, "misread {misread}");
    }

    /// 8-bit Lab hand-derived vectors: L=100 → 255, a=0 → 128, b=−128 → 0, a=127 → 255;
    /// L=50 → 127.5 → floor(+0.5) = 128 (round-half-up pin). Round trip within half a
    /// step, and decode∘encode identity over the full u8 sweep.
    #[test]
    fn lab_8_encoding_vectors_and_sweeps() {
        assert_eq!(encode_lab_8([100.0, 0.0, -128.0]), [255, 128, 0]);
        assert_eq!(encode_lab_8([50.0, 127.0, 127.0]), [128, 255, 255]);
        assert_eq!(encode_lab_8([-5.0, -300.0, 300.0]), [0, 0, 255]);
        for lab in [[0.0, 0.0, 0.0], [42.17, -27.3, 88.8]] {
            let dec = decode_lab_8(encode_lab_8(lab));
            assert!((dec[0] - lab[0]).abs() <= 0.5 * 100.0 / 255.0);
            assert!((dec[1] - lab[1]).abs() <= 0.5);
            assert!((dec[2] - lab[2]).abs() <= 0.5);
        }
        for u in 0..=u8::MAX {
            assert_eq!(encode_lab_8(decode_lab_8([u; 3])), [u; 3]);
        }
    }

    /// LCh round trips, including hue angles in every quadrant and near the 0°/360° wrap;
    /// the polar form must normalize h into [0, 360) and preserve C exactly enough for a
    /// 1e-12 round trip. Also pins the achromatic convention h = 0 at a = b = 0.
    #[test]
    fn lch_round_trips_and_hue_wrap() {
        let cases = [
            [50.0, 10.0, 10.0],   // 45°
            [50.0, -10.0, 10.0],  // 135°
            [50.0, -10.0, -10.0], // 225°
            [50.0, 10.0, -10.0],  // 315°
            [50.0, 30.0, -1e-9],  // just below 360° after normalization
            [50.0, 30.0, 1e-9],   // just above 0°
        ];
        for lab in cases {
            let lch = lab_to_lch(lab);
            assert!(
                (0.0..360.0).contains(&lch[2]),
                "h out of [0,360): {}",
                lch[2]
            );
            let rt = lch_to_lab(lch);
            for i in 0..3 {
                assert!((rt[i] - lab[i]).abs() < 1e-12, "{lab:?} → {lch:?} → {rt:?}");
            }
        }
        // Hue wrap on the way back in: h and h + 360 are the same colour.
        let a = lch_to_lab([50.0, 20.0, 350.0]);
        let b = lch_to_lab([50.0, 20.0, 710.0]);
        for i in 0..3 {
            assert!((a[i] - b[i]).abs() < 1e-9);
        }
        // Achromatic convention.
        assert_eq!(lab_to_lch([42.0, 0.0, 0.0]), [42.0, 0.0, 0.0]);
    }

    /// xyY round trips over in-gamut colours, plus both documented black conventions:
    /// X+Y+Z == 0 → x = y = 0 sentinel, and y == 0 → XYZ black. A mutated numerator or
    /// missing sentinel divides by zero and fails on NaN.
    #[test]
    fn xyy_round_trips_and_black_convention() {
        for xyz in [[0.5, 0.7, 0.2], D50_XYZ, D65_XYZ, [0.01, 0.02, 0.03]] {
            let rt = xyy_to_xyz(xyz_to_xyy(xyz));
            for i in 0..3 {
                assert!((rt[i] - xyz[i]).abs() < 1e-12, "{xyz:?} → {rt:?}");
            }
        }
        assert_eq!(xyz_to_xyy([0.0, 0.0, 0.0]), [0.0, 0.0, 0.0]);
        assert_eq!(xyy_to_xyz(xyz_to_xyy([0.0, 0.0, 0.0])), [0.0, 0.0, 0.0]);
        // y == 0 with non-zero x also maps to black (alychne convention).
        assert_eq!(xyy_to_xyz([0.3, 0.0, 0.5]), [0.0, 0.0, 0.0]);
    }

    /// D50 sanity against the crate's ICC sibling: the exact rationals must round-trip
    /// the header's u1Fixed15 raw values (63190, 54061 over 65536) — a rounded 0.9642
    /// literal (the lcms2 shortcut) is ≈3e-6 off and fails the exact comparison.
    #[test]
    fn d50_xyz_is_the_exact_icc_header_rational() {
        assert_eq!(D50_XYZ[0] * 65536.0, 63190.0);
        assert_eq!(D50_XYZ[1], 1.0);
        assert_eq!(D50_XYZ[2] * 65536.0, 54061.0);
    }
}
