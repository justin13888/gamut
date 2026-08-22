//! Standalone lcms2 tone curves (`cmsToneCurve`): construction (pure gamma, parametric,
//! tabulated), evaluation, and numeric/analytic reversal — both to feed the profile synthesizers
//! in [`crate::synth`] and to cross-check `gamut-color`'s curve mathematics directly.

use std::ptr;

use crate::sys;

/// An owned lcms2 tone curve, freed on drop.
pub struct ToneCurve(pub(crate) *mut sys::cmsToneCurve);

impl ToneCurve {
    /// A pure power-law curve `y = x^g` (`cmsBuildGamma`).
    #[must_use]
    pub fn gamma(g: f64) -> Self {
        // SAFETY: global context (null) is valid; returns an owned curve (checked non-null below).
        let p = unsafe { sys::cmsBuildGamma(ptr::null_mut(), g) };
        assert!(!p.is_null(), "cmsBuildGamma returned null");
        Self(p)
    }

    /// A parametric curve of one of lcms2's built-in types (`cmsBuildParametricToneCurve`).
    ///
    /// `params` must carry exactly the parameter count of `curve_type` (ICC type + 1 convention;
    /// a negative type is the analytic inverse): 1→1, 2→3, 3→4, 4→5 (the sRGB shape), 5→7,
    /// 6→4, 7→5, 8→5, 108→1, 109→1.
    #[must_use]
    pub fn parametric(curve_type: i32, params: &[f64]) -> Self {
        let expected = match curve_type.abs() {
            1 | 108 | 109 => 1,
            2 => 3,
            3 | 6 => 4,
            4 | 7 | 8 => 5,
            5 => 7,
            other => panic!("unknown lcms2 parametric curve type {other}"),
        };
        assert_eq!(
            params.len(),
            expected,
            "parametric type {curve_type} takes {expected} params"
        );
        // SAFETY: `params` holds the exact count lcms2 reads for this type (asserted above).
        let p = unsafe {
            sys::cmsBuildParametricToneCurve(ptr::null_mut(), curve_type, params.as_ptr())
        };
        assert!(!p.is_null(), "cmsBuildParametricToneCurve returned null");
        Self(p)
    }

    /// A sampled 16-bit curve over `values` taken as equally-spaced samples of `[0, 1] → [0, 1]`
    /// (`cmsBuildTabulatedToneCurve16`).
    #[must_use]
    pub fn tabulated_u16(values: &[u16]) -> Self {
        let n = u32::try_from(values.len()).expect("table length fits u32");
        // SAFETY: `values` is valid for `n` entries; lcms copies the table into the curve.
        let p = unsafe { sys::cmsBuildTabulatedToneCurve16(ptr::null_mut(), n, values.as_ptr()) };
        assert!(!p.is_null(), "cmsBuildTabulatedToneCurve16 returned null");
        Self(p)
    }

    /// A sampled float curve over `values` taken as equally-spaced samples of `[0, 1]`
    /// (`cmsBuildTabulatedToneCurveFloat`).
    #[must_use]
    pub fn tabulated_f32(values: &[f32]) -> Self {
        let n = u32::try_from(values.len()).expect("table length fits u32");
        // SAFETY: `values` is valid for `n` entries; lcms copies the table into the curve.
        let p =
            unsafe { sys::cmsBuildTabulatedToneCurveFloat(ptr::null_mut(), n, values.as_ptr()) };
        assert!(
            !p.is_null(),
            "cmsBuildTabulatedToneCurveFloat returned null"
        );
        Self(p)
    }

    /// Evaluate the curve at `x` (`cmsEvalToneCurveFloat`). Note lcms2 quantizes table-only
    /// curves to 16 bits even on this float path; parametric/segmented curves evaluate in float.
    #[must_use]
    pub fn eval_f32(&self, x: f32) -> f32 {
        // SAFETY: `0` is a live curve owned by `self`.
        unsafe { sys::cmsEvalToneCurveFloat(self.0, x) }
    }

    /// The functional inverse (`cmsReverseToneCurveEx`): analytic when the curve is a single
    /// registered parametric segment, otherwise a numerically reversed `samples`-entry table.
    #[must_use]
    pub fn reversed(&self, samples: u32) -> ToneCurve {
        // SAFETY: `0` is a live curve; the result is a new owned curve.
        let p = unsafe { sys::cmsReverseToneCurveEx(samples, self.0) };
        assert!(!p.is_null(), "cmsReverseToneCurveEx returned null");
        ToneCurve(p)
    }

    /// Whether the curve is overall descending, i.e. `f(0) > f(1)` on the 16-bit shadow table
    /// (`cmsIsToneCurveDescending`).
    #[must_use]
    pub fn is_descending(&self) -> bool {
        // SAFETY: `0` is a live curve owned by `self`.
        unsafe { sys::cmsIsToneCurveDescending(self.0) != 0 }
    }
}

impl Drop for ToneCurve {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `0` is an owned curve from an lcms2 constructor, freed exactly once.
            unsafe { sys::cmsFreeToneCurve(self.0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The true sRGB piecewise EOTF as a parametric type-4 curve: hand-evaluated at the linear
    /// segment, the junction, and the power segment.
    #[test]
    fn parametric_type_4_is_srgb() {
        let srgb =
            ToneCurve::parametric(4, &[2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045]);
        // Linear segment: x < 0.04045 ⇒ y = x / 12.92.
        let y = f64::from(srgb.eval_f32(0.02));
        assert!((y - 0.02 / 12.92).abs() < 1e-6, "linear segment: {y}");
        // Power segment: y = ((x + 0.055)/1.055)^2.4.
        let want = (0.5f64 + 0.055) / 1.055;
        let want = want.powf(2.4);
        let y = f64::from(srgb.eval_f32(0.5));
        assert!((y - want).abs() < 1e-6, "power segment: {y} vs {want}");
    }

    /// A gamma curve's reverse is the reciprocal gamma (analytic inversion path), so
    /// forward∘reverse is the identity well inside float tolerance.
    #[test]
    fn reversed_gamma_round_trips() {
        let g = ToneCurve::gamma(2.2);
        let inv = g.reversed(4096);
        for i in 0..=16 {
            let x = i as f32 / 16.0;
            let rt = inv.eval_f32(g.eval_f32(x));
            assert!((rt - x).abs() < 1e-4, "round trip at {x}: {rt}");
        }
    }

    /// Tabulated constructors: a descending 16-bit ramp reports `is_descending`, an ascending
    /// float ramp does not, and both evaluate their endpoints.
    #[test]
    fn tabulated_curves_and_descent() {
        let down = ToneCurve::tabulated_u16(&[0xFFFF, 0x8000, 0]);
        assert!(down.is_descending());
        assert!(down.eval_f32(0.0) > 0.99);
        assert!(down.eval_f32(1.0) < 0.01);

        let up = ToneCurve::tabulated_f32(&[0.0, 0.25, 0.5, 0.75, 1.0]);
        assert!(!up.is_descending());
        let mid = up.eval_f32(0.5);
        assert!((mid - 0.5).abs() < 1e-3, "midpoint {mid}");
    }
}
