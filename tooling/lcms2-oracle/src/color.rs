//! Scalar colorimetry entry points: ΔE metrics (`cmsDeltaE`, `cmsCIE2000DeltaE`), XYZ↔Lab
//! conversion (`cmsXYZ2Lab`/`cmsLab2XYZ`), and the fixed-point PCS encoders
//! (`cmsFloat2LabEncoded[V2]`, `cmsFloat2XYZEncoded`, and their decoders) — the reference side of
//! `gamut-color::lab`'s differential tests.

use std::ptr;

use crate::sys;

fn to_lab(lab: [f64; 3]) -> sys::cmsCIELab {
    sys::cmsCIELab {
        L: lab[0],
        a: lab[1],
        b: lab[2],
    }
}

fn to_xyz(xyz: [f64; 3]) -> sys::cmsCIEXYZ {
    sys::cmsCIEXYZ {
        X: xyz[0],
        Y: xyz[1],
        Z: xyz[2],
    }
}

/// CIE XYZ → CIE L\*a\*b\* (`cmsXYZ2Lab`) relative to `white`; `None` selects lcms2's built-in
/// D50 (the *rounded* `0.9642/1.0/0.8249` literals, ≈ 3e-6 away from the ICC header rationals).
#[must_use]
pub fn xyz_to_lab(white: Option<[f64; 3]>, xyz: [f64; 3]) -> [f64; 3] {
    let white = white.map(to_xyz);
    let white_ptr = white.as_ref().map_or(ptr::null(), |w| w as *const _);
    let xyz = to_xyz(xyz);
    let mut lab = to_lab([0.0; 3]);
    // SAFETY: all pointers are live locals (or null, which lcms documents as D50).
    unsafe { sys::cmsXYZ2Lab(white_ptr, &mut lab, &xyz) };
    [lab.L, lab.a, lab.b]
}

/// CIE L\*a\*b\* → CIE XYZ (`cmsLab2XYZ`) relative to `white`; `None` selects lcms2's built-in
/// D50 (see [`xyz_to_lab`]).
#[must_use]
pub fn lab_to_xyz(white: Option<[f64; 3]>, lab: [f64; 3]) -> [f64; 3] {
    let white = white.map(to_xyz);
    let white_ptr = white.as_ref().map_or(ptr::null(), |w| w as *const _);
    let lab = to_lab(lab);
    let mut xyz = to_xyz([0.0; 3]);
    // SAFETY: all pointers are live locals (or null, which lcms documents as D50).
    unsafe { sys::cmsLab2XYZ(white_ptr, &mut xyz, &lab) };
    [xyz.X, xyz.Y, xyz.Z]
}

/// CIE76 colour difference ΔE\*ab (`cmsDeltaE`): the Euclidean distance between two Lab colours.
#[must_use]
pub fn delta_e_76(lab1: [f64; 3], lab2: [f64; 3]) -> f64 {
    let (lab1, lab2) = (to_lab(lab1), to_lab(lab2));
    // SAFETY: both pointers are live locals.
    unsafe { sys::cmsDeltaE(&lab1, &lab2) }
}

/// CIEDE2000 colour difference ΔE₀₀ (`cmsCIE2000DeltaE`) with parametric weights `kl`/`kc`/`kh`
/// (all 1 for reference conditions).
#[must_use]
pub fn cie2000_delta_e(lab1: [f64; 3], lab2: [f64; 3], kl: f64, kc: f64, kh: f64) -> f64 {
    let (lab1, lab2) = (to_lab(lab1), to_lab(lab2));
    // SAFETY: both pointers are live locals.
    unsafe { sys::cmsCIE2000DeltaE(&lab1, &lab2, kl, kc, kh) }
}

/// Encode Lab into the ICC **v4** 16-bit PCSLAB encoding (`cmsFloat2LabEncoded`).
#[must_use]
pub fn lab_encode_v4(lab: [f64; 3]) -> [u16; 3] {
    let lab = to_lab(lab);
    let mut w = [0u16; 3];
    // SAFETY: `w` has the 3 entries lcms writes.
    unsafe { sys::cmsFloat2LabEncoded(w.as_mut_ptr(), &lab) };
    w
}

/// Decode the ICC **v4** 16-bit PCSLAB encoding (`cmsLabEncoded2Float`).
#[must_use]
pub fn lab_decode_v4(w: [u16; 3]) -> [f64; 3] {
    let mut lab = to_lab([0.0; 3]);
    // SAFETY: `w` has the 3 entries lcms reads.
    unsafe { sys::cmsLabEncoded2Float(&mut lab, w.as_ptr()) };
    [lab.L, lab.a, lab.b]
}

/// Encode Lab into the legacy **v2** 16-bit PCSLAB encoding (`cmsFloat2LabEncodedV2`).
#[must_use]
pub fn lab_encode_v2(lab: [f64; 3]) -> [u16; 3] {
    let lab = to_lab(lab);
    let mut w = [0u16; 3];
    // SAFETY: `w` has the 3 entries lcms writes.
    unsafe { sys::cmsFloat2LabEncodedV2(w.as_mut_ptr(), &lab) };
    w
}

/// Decode the legacy **v2** 16-bit PCSLAB encoding (`cmsLabEncoded2FloatV2`).
#[must_use]
pub fn lab_decode_v2(w: [u16; 3]) -> [f64; 3] {
    let mut lab = to_lab([0.0; 3]);
    // SAFETY: `w` has the 3 entries lcms reads.
    unsafe { sys::cmsLabEncoded2FloatV2(&mut lab, w.as_ptr()) };
    [lab.L, lab.a, lab.b]
}

/// Encode XYZ into the PCSXYZ u1Fixed15 encoding (`cmsFloat2XYZEncoded`), including lcms2's
/// `Y <= 0 ⇒ all-zero` rule.
#[must_use]
pub fn xyz_encode(xyz: [f64; 3]) -> [u16; 3] {
    let xyz = to_xyz(xyz);
    let mut w = [0u16; 3];
    // SAFETY: `w` has the 3 entries lcms writes.
    unsafe { sys::cmsFloat2XYZEncoded(w.as_mut_ptr(), &xyz) };
    w
}

/// Decode the PCSXYZ u1Fixed15 encoding (`cmsXYZEncoded2Float`): exactly `v / 32768`.
#[must_use]
pub fn xyz_decode(w: [u16; 3]) -> [f64; 3] {
    let mut xyz = to_xyz([0.0; 3]);
    // SAFETY: `w` has the 3 entries lcms reads.
    unsafe { sys::cmsXYZEncoded2Float(&mut xyz, w.as_ptr()) };
    [xyz.X, xyz.Y, xyz.Z]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-derived pins so the wrappers demonstrably reach the intended entry points (the heavy
    /// differential sweeps live in `gamut-color/tests/lab_oracle.rs`).
    #[test]
    fn wrappers_reach_lcms_entry_points() {
        // v4 white: L=100 → 0xFFFF, a=b=0 → 0x8080. v2 white: L=100 → 0xFF00, a=b=0 → 0x8000.
        assert_eq!(lab_encode_v4([100.0, 0.0, 0.0]), [0xFFFF, 0x8080, 0x8080]);
        assert_eq!(lab_encode_v2([100.0, 0.0, 0.0]), [0xFF00, 0x8000, 0x8000]);
        assert_eq!(xyz_encode([1.0, 1.0, 1.0]), [0x8000, 0x8000, 0x8000]);
        assert_eq!(xyz_encode([1.0, 0.0, 1.0]), [0, 0, 0], "Y<=0 zeroes all");
        let lab = lab_decode_v4([0xFFFF, 0x8080, 0x8080]);
        assert!((lab[0] - 100.0).abs() < 1e-9 && lab[1].abs() < 1e-9);
        let xyz = xyz_decode([0x8000, 0x8000, 0x8000]);
        assert_eq!(xyz, [1.0, 1.0, 1.0]);
        // ΔE76 of a 3-4-5 triangle; ΔE00 of an identical pair is 0.
        let de = delta_e_76([50.0, 3.0, 4.0], [50.0, 0.0, 0.0]);
        assert!((de - 5.0).abs() < 1e-12);
        assert_eq!(
            cie2000_delta_e([50.0, 1.0, -2.0], [50.0, 1.0, -2.0], 1.0, 1.0, 1.0),
            0.0
        );
        // Explicit-white XYZ↔Lab round trip through lcms.
        let white = [0.9642, 1.0, 0.8249];
        let lab = xyz_to_lab(Some(white), white);
        assert!((lab[0] - 100.0).abs() < 1e-9 && lab[1].abs() < 1e-9);
        let rt = lab_to_xyz(Some(white), lab);
        for k in 0..3 {
            assert!((rt[k] - white[k]).abs() < 1e-9);
        }
        // NULL white is lcms's own D50: same result as passing its literals explicitly.
        let a = xyz_to_lab(None, [0.3, 0.4, 0.2]);
        let b = xyz_to_lab(Some(white), [0.3, 0.4, 0.2]);
        for k in 0..3 {
            assert!((a[k] - b[k]).abs() < 1e-12);
        }
    }
}
