//! Encoder configuration: lossless vs. lossy selection and the quality knob.

/// Which WebP bitstream the encoder produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
/// input exactly and currently ignores `quality`; tuning lossless compression density (a possible
/// future effort knob) is tracked in issue #31.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebpConfig {
    /// The bitstream mode to encode.
    pub mode: WebpMode,
    /// Lossy quality factor, `0..=100` (higher = larger, closer to the source). Ignored for lossless.
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
