//! Encoder configuration: lossless vs. lossy selection and the quality knob.

/// Which WebP bitstream the encoder produces.
///
/// `#[non_exhaustive]`: modes are an open set — variants for deferred coding strategies are added
/// as they ship, so match with a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum WebpMode {
    /// VP8L lossless coding — the input is reproduced bit-exactly (the default; gamut's M0 path).
    #[default]
    Lossless,
    /// VP8 lossy coding — smaller output at a quality/size tradeoff set by [`WebpConfig::quality`].
    Lossy,
}

/// Configuration for a [`WebpEncoder`](crate::WebpEncoder).
///
/// `quality` ranges `0..=100` and applies only to [`WebpMode::Lossy`], where it is the usual quality
/// factor (higher = larger output, closer to the source). [`WebpMode::Lossless`] reproduces the
/// input exactly and ignores `quality`. Build one with the [`WebpEncoder`](crate::WebpEncoder)
/// constructors and builders rather than by hand — they keep the fields consistent.
///
/// `#[non_exhaustive]`: the configuration is an open set — fields for deferred encoder knobs are
/// added as they ship. Read it as the snapshot returned by
/// [`WebpEncoder::config`](crate::WebpEncoder::config).
///
/// `Copy` is **retained** deliberately, unlike [`AvifConfig`](https://docs.rs/gamut-avif): the
/// owned payloads that forced AVIF to drop it (ICC profiles, Exif/XMP) already live on
/// [`WebpEncoder`](crate::WebpEncoder) rather than in the config, and every knob here is plain
/// scalar data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct WebpConfig {
    /// The bitstream mode to encode.
    pub mode: WebpMode,
    /// Lossy quality factor, `0..=100` (higher = larger, closer to the source). Ignored for
    /// lossless. Values above `100` behave as `100` (the encoder clamps silently) — a frozen
    /// contract.
    pub quality: u8,
}

impl Default for WebpConfig {
    fn default() -> Self {
        Self {
            mode: WebpMode::Lossless,
            quality: 75,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_lossless_quality_75() {
        let c = WebpConfig::default();
        assert_eq!(c.mode, WebpMode::Lossless);
        assert_eq!(c.quality, 75);
        assert_eq!(WebpMode::default(), WebpMode::Lossless);
    }
}
