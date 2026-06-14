//! Reference luminance levels for HDR/SDR signal handling, shared across the colour and
//! tone-mapping crates so a single definition is authoritative.
//!
//! Values are in candela per square metre (cd/m², "nits"). Provenance is recorded in
//! `references/color/README.md` (Report ITU-R BT.2408 — HDR Reference White; SMPTE ST 2084 /
//! ITU-R BT.2100 — PQ peak).

/// SDR diffuse-white reference luminance (cd/m²): the classic 100-nit reference.
///
/// ITU-R BT.2408 is the framework that maps SDR 100 % diffuse white onto the HDR reference level
/// ([`HDR_REFERENCE_WHITE_NITS`]).
pub const SDR_REFERENCE_WHITE_NITS: f64 = 100.0;

/// HDR Reference White (cd/m²): 203, per Report ITU-R BT.2408.
///
/// The nominal signal level of graphics / diffuse white in PQ and HLG production. (Not "SDR" white —
/// SDR diffuse white is [`SDR_REFERENCE_WHITE_NITS`].)
pub const HDR_REFERENCE_WHITE_NITS: f64 = 203.0;

/// PQ peak luminance (cd/m²): the 10 000-nit maximum of SMPTE ST 2084 / ITU-R BT.2100.
pub const PQ_PEAK_NITS: f64 = 10_000.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_levels_match_standards() {
        // Pin the literals against their standards so an accidental edit is caught.
        assert_eq!(SDR_REFERENCE_WHITE_NITS, 100.0);
        assert_eq!(HDR_REFERENCE_WHITE_NITS, 203.0);
        assert_eq!(PQ_PEAK_NITS, 10_000.0);
        // HDR Reference White is 2.03x the SDR reference (the gamut-tonemap default white point).
        assert!((HDR_REFERENCE_WHITE_NITS / SDR_REFERENCE_WHITE_NITS - 2.03).abs() < 1e-12);
    }
}
