//! Per-operator parameter defaults for the built-in tone curves.
//!
//! Absolute colour-science reference luminances (SDR/HDR white in nits, PQ peak) live in
//! [`gamut_core::luminance`], the single authoritative home shared with `gamut-color`; see
//! `references/tonemap/README.md` for the operators these defaults parameterize.

/// Normalized SDR display white in linear units (`1.0`). Default upper bound for
/// [`Clamp`](crate::operators::Clamp).
pub const SDR_WHITE_NORMALIZED: f32 = 1.0;

/// Default white point for [`ReinhardExtended`](crate::operators::ReinhardExtended) when none is
/// given: the HDR-to-SDR reference-white ratio — BT.2408 HDR reference white (203 cd/m²) over the
/// classic SDR diffuse white (100 cd/m²), i.e. `203 / 100 = 2.03`. See [`gamut_core::luminance`].
pub const DEFAULT_REINHARD_WHITE: f32 = 2.03;

#[cfg(test)]
mod tests {
    #[test]
    fn default_reinhard_white_matches_core_ratio() {
        // Pin the literal to the authoritative gamut-core luminance constants so a drift in either
        // is caught (the literal avoids an f64->f32 cast in the public const).
        let ratio = (gamut_core::luminance::HDR_REFERENCE_WHITE_NITS
            / gamut_core::luminance::SDR_REFERENCE_WHITE_NITS) as f32;
        assert_eq!(super::DEFAULT_REINHARD_WHITE, ratio);
    }
}
