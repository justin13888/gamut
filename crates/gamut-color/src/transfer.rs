//! Transfer functions (EOTF / inverse-EOTF) in `f64`, covering the gamuts the
//! gamut ecosystem encodes through.
//!
//! Two flavours are exposed for the curves that have a *deliberately simplified*
//! encoder form (see `references/color/README.md`):
//!
//! * **encoder-exact** — what the encoder actually applied, so a metrics tool
//!   predicts the same bitstream: Adobe RGB is pure `x^2.2` (not the standard
//!   `x^2.19921875`), ProPhoto RGB is pure `x^1.8` (no linear toe), and the
//!   BT.2020 path is PQ inverse-EOTF → nits → Reinhard@203 (tone-mapped to SDR).
//! * **standard** — the textbook curve, reachable via the `*_standard` helpers.
//!
//! Determinism is **Tier-1** (correctness only): these use `std` `f64::powf`, not
//! a bit-reproducible substrate. Inputs are assumed in `[0, 1]`; the curves are
//! evaluated as written (no clamping) so they stay faithful to the reference.

use crate::cicp::TransferCharacteristics;

/// HDR reference white — the graphics / diffuse-white luminance (cd/m²), per ITU-R BT.2408. The
/// BT.2020 PQ path tone-maps relative to this level. Named to match the same-value constant in the
/// `gamut-tonemap` crate (this crate keeps an `f64` copy for its Tier-1 encoder-exact path).
pub const HDR_REFERENCE_WHITE_NITS: f64 = 203.0;
/// PQ peak luminance (cd/m²), per SMPTE ST 2084.
pub const PQ_PEAK_NITS: f64 = 10_000.0;

// --- sRGB (IEC 61966-2-1) --------------------------------------------------

/// sRGB EOTF: gamma-encoded signal → linear light, on `[0, 1]`.
///
/// # Examples
///
/// ```
/// use gamut_color::transfer::srgb_eotf;
/// assert_eq!(srgb_eotf(0.0), 0.0);
/// assert!((srgb_eotf(1.0) - 1.0).abs() < 1e-12);
/// // sRGB 0.5 is ~0.214 in linear light.
/// assert!((srgb_eotf(0.5) - 0.214).abs() < 1e-3);
/// ```
#[must_use]
pub fn srgb_eotf(x: f64) -> f64 {
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}

/// sRGB inverse EOTF (the "gamma" encode): linear light → gamma-encoded signal.
#[must_use]
pub fn srgb_oetf(x: f64) -> f64 {
    if x <= 0.0031308 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

// --- Adobe RGB (1998) ------------------------------------------------------

/// Adobe RGB EOTF, **encoder-exact** simplification: pure `x^2.2`.
#[must_use]
pub fn adobe_rgb_eotf(x: f64) -> f64 {
    x.powf(2.2)
}

/// Adobe RGB EOTF, **standard** curve: `x^(563/256)` = `x^2.19921875`.
#[must_use]
pub fn adobe_rgb_eotf_standard(x: f64) -> f64 {
    x.powf(563.0 / 256.0)
}

// --- ProPhoto RGB / ROMM (ISO 22028-2) -------------------------------------

/// ProPhoto RGB EOTF, **encoder-exact** simplification: pure `x^1.8` (no toe).
#[must_use]
pub fn prophoto_rgb_eotf(x: f64) -> f64 {
    x.powf(1.8)
}

/// ProPhoto RGB EOTF, **standard** ROMM curve: linear toe of slope 16 below
/// `16·Eₜ = 1/32`, then `x^1.8`.
#[must_use]
pub fn prophoto_rgb_eotf_standard(x: f64) -> f64 {
    if x < 1.0 / 32.0 {
        x / 16.0
    } else {
        x.powf(1.8)
    }
}

// --- BT.2020 PQ (SMPTE ST 2084 / ITU-R BT.2100) ----------------------------

/// PQ (ST 2084) inverse EOTF: gamma-encoded signal → **absolute luminance in
/// cd/m² (nits)**, in `[0, 10000]`. This is the standards-pure curve, with no
/// tone mapping. Use [`bt2020_pq_to_sdr`] for the encoder-exact BT.2020 path.
#[must_use]
pub fn pq_eotf(x: f64) -> f64 {
    // ST 2084 constants (exact dyadic rationals; see references/color/README.md).
    const M1: f64 = 0.1593017578125;
    const M2: f64 = 78.84375;
    const C1: f64 = 0.8359375;
    const C2: f64 = 18.8515625;
    const C3: f64 = 18.6875;

    let n = x.powf(1.0 / M2);
    let num = (n - C1).max(0.0);
    let den = C2 - C3 * n;
    let y_normalized = (num / den).powf(1.0 / M1);
    y_normalized * PQ_PEAK_NITS
}

/// BT.2020 **encoder-exact** path: PQ inverse EOTF → nits → Reinhard tone map to
/// SDR `[0, 1)` relative to the 203-nit reference white (`L / (1 + L)`).
///
/// The `L / (1 + L)` step is the basic Reinhard operator. For general-purpose tone-curve operators
/// on linear light (Reinhard, extended Reinhard, …) see the separate `gamut-tonemap` crate; this is
/// the `f64` curve baked into the BT.2020 transfer so a metrics tool can predict the encoder's exact
/// output (`gamut-tonemap` is the `f32` general toolkit, this is the Tier-1 encoder-exact path).
#[must_use]
pub fn bt2020_pq_to_sdr(x: f64) -> f64 {
    let l = pq_eotf(x) / HDR_REFERENCE_WHITE_NITS;
    l / (1.0 + l)
}

// --- CICP dispatch ---------------------------------------------------------

/// The encoder-exact EOTF for a CICP [`TransferCharacteristics`] code point, as
/// a `fn` mapping signal → scene/display-linear `[0, 1]`.
///
/// `Srgb` returns scene-linear via [`srgb_eotf`]; `Pq` / `Bt2020_10` return the
/// tone-mapped SDR value via [`bt2020_pq_to_sdr`]. Code points without a curve
/// implemented here (`Bt709`, `Hlg`, `Unspecified`) return `None`. Adobe RGB and
/// ProPhoto have no CICP transfer code point and are reached via their named
/// functions or [`crate::profile`].
#[must_use]
pub fn eotf_for(tc: TransferCharacteristics) -> Option<fn(f64) -> f64> {
    match tc {
        TransferCharacteristics::Srgb => Some(srgb_eotf),
        TransferCharacteristics::Pq | TransferCharacteristics::Bt2020_10 => Some(bt2020_pq_to_sdr),
        TransferCharacteristics::Bt709
        | TransferCharacteristics::Hlg
        | TransferCharacteristics::Unspecified => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_roundtrip() {
        for &x in &[0.0, 0.01, 0.04045, 0.1, 0.5, 0.9, 1.0] {
            let back = srgb_oetf(srgb_eotf(x));
            // Tolerance accounts for the ~3e-8 discontinuity the sRGB standard
            // bakes into its two piecewise breakpoints (0.04045 vs 0.0031308·12.92).
            assert!((back - x).abs() < 1e-4, "sRGB roundtrip at {x}: {back}");
        }
    }

    #[test]
    fn srgb_piecewise_continuous_at_thresholds() {
        assert!((srgb_eotf(0.04045) - 0.04045 / 12.92).abs() < 1e-12);
        assert!((srgb_oetf(0.0031308) - 12.92 * 0.0031308).abs() < 1e-12);
    }

    #[test]
    fn boundaries_map_zero_and_one() {
        for f in [
            srgb_eotf as fn(f64) -> f64,
            srgb_oetf,
            adobe_rgb_eotf,
            adobe_rgb_eotf_standard,
            prophoto_rgb_eotf,
            prophoto_rgb_eotf_standard,
        ] {
            assert_eq!(f(0.0), 0.0);
            assert!((f(1.0) - 1.0).abs() < 1e-12, "f(1.0) should be 1.0");
        }
    }

    #[test]
    fn adobe_and_prophoto_encoder_exact_vs_standard_differ() {
        // The simplified curves must measurably differ from the textbook ones,
        // or the "selectable" distinction is meaningless.
        let x = 0.5;
        assert!((adobe_rgb_eotf(x) - adobe_rgb_eotf_standard(x)).abs() > 1e-4);
        assert!((prophoto_rgb_eotf(0.02) - prophoto_rgb_eotf_standard(0.02)).abs() > 1e-4);
        // Pin the *exact* standard exponent: a mutated `563 / 256` (e.g. `563 % 256` or `563 * 256`)
        // still "differs" from x^2.2, so the inequality above can't see it.
        assert!((adobe_rgb_eotf_standard(0.5) - 0.5_f64.powf(563.0 / 256.0)).abs() < 1e-15);
    }

    #[test]
    fn prophoto_standard_toe_is_linear() {
        // Below 1/32 the standard curve is exactly x/16.
        assert!((prophoto_rgb_eotf_standard(1.0 / 64.0) - (1.0 / 64.0) / 16.0).abs() < 1e-15);
        // The toe meets the power curve exactly at the x = 1/32 breakpoint — both are 2^-9, since
        // 32 = 2^5 and 5 * 1.8 = 9 — so the curve is continuous there. That continuity is precisely
        // why `<` vs `<=` at the threshold is unobservable (an equivalent mutant, excluded in
        // .cargo/mutants.toml). Above the breakpoint the curve is x^1.8; x = 0.5 pins the power side
        // and catches a mutated `1.0 / 32.0` threshold (e.g. `1.0 % 32.0 == 1.0`, extending the toe).
        assert_eq!(
            prophoto_rgb_eotf_standard(1.0 / 32.0),
            (1.0_f64 / 32.0).powf(1.8)
        );
        assert!((prophoto_rgb_eotf_standard(0.5) - 0.5_f64.powf(1.8)).abs() < 1e-15);
    }

    #[test]
    fn pq_eotf_endpoints() {
        assert_eq!(pq_eotf(0.0), 0.0);
        // Full PQ signal decodes to peak luminance.
        assert!(
            (pq_eotf(1.0) - PQ_PEAK_NITS).abs() < 1e-6,
            "{}",
            pq_eotf(1.0)
        );
    }

    #[test]
    fn bt2020_pq_to_sdr_matches_reference_formula() {
        // Pin every arithmetic step against an independent powf reference so a
        // mutated operator diverges past 1e-12 across the curve's interior.
        fn reference(x: f64) -> f64 {
            const M1: f64 = 0.1593017578125;
            const M2: f64 = 78.84375;
            const C1: f64 = 0.8359375;
            const C2: f64 = 18.8515625;
            const C3: f64 = 18.6875;
            let n = x.powf(1.0 / M2);
            let num = (n - C1).max(0.0);
            let den = C2 - C3 * n;
            let y = (num / den).powf(1.0 / M1);
            let l = y * 10000.0 / 203.0;
            l / (1.0 + l)
        }
        for &x in &[0.0, 0.05, 0.1, 0.25, 0.5, 0.6, 0.75, 0.9, 1.0] {
            assert!((bt2020_pq_to_sdr(x) - reference(x)).abs() < 1e-12, "at {x}");
        }
    }

    #[test]
    fn bt2020_pq_full_signal_near_one() {
        let max = bt2020_pq_to_sdr(1.0);
        assert!(max > 0.9 && max < 1.0, "PQ(1.0) → {max}");
    }

    #[test]
    fn eotf_for_dispatch() {
        assert!(eotf_for(TransferCharacteristics::Srgb).is_some());
        assert!(eotf_for(TransferCharacteristics::Pq).is_some());
        assert!(eotf_for(TransferCharacteristics::Bt2020_10).is_some());
        assert!(eotf_for(TransferCharacteristics::Hlg).is_none());
        assert!(eotf_for(TransferCharacteristics::Bt709).is_none());
        // The Srgb dispatch is the sRGB EOTF.
        let f = eotf_for(TransferCharacteristics::Srgb).unwrap();
        assert_eq!(f(0.5), srgb_eotf(0.5));
    }
}
