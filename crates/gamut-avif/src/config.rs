//! Encoder configuration: lossless vs. lossy selection and the quality knob.

/// Which AVIF bitstream the encoder produces.
///
/// `#[non_exhaustive]`: modes are an open set — variants for deferred coding strategies (e.g.
/// metric- or size-targeted rate control, tracked in `STATUS.md`) are added as they ship, so
/// match with a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum AvifMode {
    /// Lossless AV1 intra coding — the decoded output is bit-exact to the input (the default;
    /// gamut's M0 path). The RGB samples map to identity-matrix 4:4:4 planes, so no colour
    /// conversion is applied.
    #[default]
    Lossless,
    /// Lossy AV1 intra coding — smaller output at a quality/size tradeoff set by
    /// [`AvifConfig::quality`].
    Lossy,
}

/// Configuration for an [`AvifEncoder`](crate::AvifEncoder).
///
/// `quality` ranges `0..=100` (higher = larger output, closer to the source) and applies to
/// [`AvifMode::Lossy`]; it is ignored for [`AvifMode::Lossless`]. Build one with the
/// [`AvifEncoder`](crate::AvifEncoder) constructors rather than by hand — they keep `mode` and
/// `quality` consistent.
///
/// `#[non_exhaustive]`: the configuration is an open set — fields for deferred encoder knobs
/// (speed, rate control, bit depth, chroma subsampling; see `STATUS.md`) are added as they ship.
/// Read it as the snapshot returned by [`AvifEncoder::config`](crate::AvifEncoder::config).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AvifConfig {
    /// The bitstream mode to encode.
    pub mode: AvifMode,
    /// Lossy quality factor, `0..=100`. Values above `100` behave as `100` (the encoder clamps
    /// silently) — a frozen v1 contract.
    pub quality: u8,
}

impl Default for AvifConfig {
    fn default() -> Self {
        Self {
            mode: AvifMode::Lossless,
            quality: 75,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_lossless_quality_75() {
        let c = AvifConfig::default();
        assert_eq!(c.mode, AvifMode::Lossless);
        assert_eq!(c.quality, 75);
        assert_eq!(AvifMode::default(), AvifMode::Lossless);
    }
}
