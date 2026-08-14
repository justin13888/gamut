//! Encoder configuration: lossless vs. lossy selection, the quality knob, and the compression
//! effort ladder.

/// Compression effort: the encode-time/output-size trade-off, from fastest ([`Effort::Fastest`])
/// to densest ([`Effort::Slowest`]).
///
/// Maps one-to-one onto libwebp's `WebPConfig::method` levels `0..=6` (`cwebp -m N`): higher
/// effort spends more time searching for a smaller file at the **same** decoded quality. Effort
/// never changes what the format guarantees — a lossless encode stays bit-exact at every level,
/// and a lossy encode keeps its [`quality`](WebpConfig::quality) target — so it is a free choice
/// that only trades time for size.
///
/// The default is [`Effort::Default`] (level 4), matching libwebp's `WebPConfigInit`.
///
/// The discriminants **are** the libwebp method numbers and are a permanent, append-only part of
/// the contract: they are what [`level`](Self::level) and [`from_level`](Self::from_level) round
/// trip, and what a numeric CLI or FFI knob carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum Effort {
    /// Level 0 — the fastest, least dense setting.
    Fastest = 0,
    /// Level 1.
    Faster = 1,
    /// Level 2.
    Fast = 2,
    /// Level 3 — one step quicker than the default.
    Moderate = 3,
    /// Level 4 — libwebp's default `method`, the balanced speed/density point.
    #[default]
    Default = 4,
    /// Level 5.
    Slower = 5,
    /// Level 6 — the slowest, densest setting.
    Slowest = 6,
}

impl Effort {
    /// The libwebp `method` level (`0..=6`) this variant selects.
    #[must_use]
    pub const fn level(self) -> u8 {
        self as u8
    }

    /// The [`Effort`] for a libwebp `method` level, or `None` if `level` is outside `0..=6`.
    ///
    /// The inverse of [`Effort::level`]; handy for wiring up a numeric CLI flag.
    #[must_use]
    pub const fn from_level(level: u8) -> Option<Self> {
        Some(match level {
            0 => Self::Fastest,
            1 => Self::Faster,
            2 => Self::Fast,
            3 => Self::Moderate,
            4 => Self::Default,
            5 => Self::Slower,
            6 => Self::Slowest,
            _ => return None,
        })
    }
}

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
    /// Compression effort (libwebp's `method`, `0..=6`). Applies to **both** modes; it never
    /// changes the decoded pixels of a lossless encode nor the quality target of a lossy one,
    /// only how hard the encoder searches and therefore how long it takes and how small the
    /// result is.
    pub effort: Effort,
}

impl Default for WebpConfig {
    fn default() -> Self {
        Self {
            mode: WebpMode::Lossless,
            quality: 75,
            effort: Effort::Default,
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
        assert_eq!(c.effort, Effort::Default);
        assert_eq!(WebpMode::default(), WebpMode::Lossless);
    }

    #[test]
    fn effort_level_round_trips_over_the_full_range() {
        // The discriminants are the libwebp `method` numbers and are a permanent part of the
        // contract, so pin both directions across the whole range plus the rejected values.
        for level in 0..=6u8 {
            let effort = Effort::from_level(level).expect("0..=6 is in range");
            assert_eq!(effort.level(), level);
        }
        assert_eq!(Effort::Fastest.level(), 0);
        assert_eq!(Effort::Default.level(), 4);
        assert_eq!(Effort::Slowest.level(), 6);
        assert_eq!(Effort::from_level(7), None);
        assert_eq!(Effort::from_level(u8::MAX), None);
        assert_eq!(Effort::default(), Effort::Default);
    }
}
