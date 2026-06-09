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
/// `quality` ranges `0..=100`. For [`WebpMode::Lossy`] it is the usual quality factor (higher =
/// larger, closer to the source); for [`WebpMode::Lossless`] it is interpreted as an effort level
/// (higher = smaller output, more work). It is ignored where a mode does not use it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebpConfig {
    /// The bitstream mode to encode.
    pub mode: WebpMode,
    /// Quality / effort, `0..=100`.
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
