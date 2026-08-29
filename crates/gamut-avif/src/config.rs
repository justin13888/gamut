//! Encoder configuration: lossless vs. lossy selection, the quality knob, and the output colour
//! encoding.

use gamut_color::{ChromaSubsampling, ColorRange, MatrixCoefficients};

/// Which AVIF bitstream the encoder produces.
///
/// `#[non_exhaustive]`: modes are an open set — variants for deferred coding strategies (e.g.
/// metric- or size-targeted rate control, tracked in `STATUS.md`) are added as they ship, so
/// match with a wildcard arm.
///
/// `#[repr(u8)]` with permanent, append-only discriminants: the mode is plain configuration data
/// that must stay mechanically portable to C (see the workspace charter).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum AvifMode {
    /// Lossless AV1 intra coding — the decoded output is bit-exact to the input (the default;
    /// gamut's M0 path). The RGB samples map to identity-matrix 4:4:4 planes, so no colour
    /// conversion is applied.
    #[default]
    Lossless = 0,
    /// Lossy AV1 intra coding — smaller output at a quality/size tradeoff set by
    /// [`AvifConfig::quality`].
    Lossy = 1,
}

/// Configuration for an [`AvifEncoder`](crate::AvifEncoder).
///
/// `quality` ranges `0..=100` (higher = larger output, closer to the source) and applies to
/// [`AvifMode::Lossy`]; it is ignored for [`AvifMode::Lossless`]. Build one with the
/// [`AvifEncoder`](crate::AvifEncoder) constructors rather than by hand — they keep `mode`,
/// `quality` and `matrix` consistent.
///
/// `#[non_exhaustive]`: the configuration is an open set — fields for deferred encoder knobs
/// (speed, rate control, bit depth; see `STATUS.md`) are added as they ship.
/// Read it as the snapshot returned by [`AvifEncoder::config`](crate::AvifEncoder::config).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct AvifConfig {
    /// The bitstream mode to encode.
    pub mode: AvifMode,
    /// Lossy quality factor, `0..=100`. Values above `100` behave as `100` (the encoder clamps
    /// silently) — a frozen v1 contract.
    pub quality: u8,
    /// The CICP matrix the samples are coded through.
    ///
    /// [`MatrixCoefficients::Identity`] carries R'G'B' directly as GBR planes — the only way to be
    /// bit-exact, so [`AvifMode::Lossless`] always uses it and **ignores** this field, exactly as
    /// it ignores `quality`. [`AvifMode::Lossy`] defaults to [`MatrixCoefficients::Bt709`], whose
    /// luma–chroma decorrelation is worth a substantial fraction of the bitrate.
    ///
    pub matrix: MatrixCoefficients,
    /// The signal range the coded samples occupy. Full range is the AVIF ecosystem's default (it
    /// is what `avifImageCreate` sets) and spends all 256 codes on the signal.
    pub range: ColorRange,
    /// How the chroma planes are sampled relative to luma.
    ///
    /// [`AvifMode::Lossy`] defaults to [`ChromaSubsampling::Cs420`]: it is smaller at equal quality
    /// on photographic content, and it is the only format a Main-profile decoder can read — 4:4:4
    /// is AV1 **Profile 1**, which several hardware still-image paths reject outright.
    ///
    /// [`AvifMode::Lossless`] always uses [`ChromaSubsampling::Cs444`] and **ignores** this field,
    /// exactly as it ignores `matrix` and `quality`: discarding three quarters of the chroma
    /// samples is not lossless, and AV1 §6.4.2 forbids the identity matrix below 4:4:4 anyway.
    ///
    /// [`ChromaSubsampling::Cs400`] (monochrome) is rejected at encode time.
    pub chroma: ChromaSubsampling,
}

impl Default for AvifConfig {
    fn default() -> Self {
        Self {
            mode: AvifMode::Lossless,
            quality: 75,
            matrix: MatrixCoefficients::Identity,
            range: ColorRange::Full,
            chroma: ChromaSubsampling::Cs444,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_lossless_identity_quality_75() {
        let c = AvifConfig::default();
        assert_eq!(c.mode, AvifMode::Lossless);
        assert_eq!(c.quality, 75);
        assert_eq!(c.matrix, MatrixCoefficients::Identity);
        assert_eq!(c.range, ColorRange::Full);
        assert_eq!(AvifMode::default(), AvifMode::Lossless);
    }

    #[test]
    fn mode_discriminants_are_the_frozen_c_abi_values() {
        // `AvifMode` is plain configuration data crossing the C boundary, so its discriminants are
        // append-only: a reordering would silently change what an existing C caller selects.
        assert_eq!(AvifMode::Lossless as u8, 0);
        assert_eq!(AvifMode::Lossy as u8, 1);
    }
}
