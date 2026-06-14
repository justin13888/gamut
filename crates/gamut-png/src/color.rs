//! PNG colour types and their bit-depth matrix (PNG spec §11.2.2, Table 12).

/// A PNG colour type, naming both the channel layout and its IHDR code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorType {
    /// Greyscale: one channel. Valid bit depths 1, 2, 4, 8, 16.
    Grayscale,
    /// Truecolour RGB: three channels. Valid bit depths 8, 16.
    Truecolor,
    /// Indexed (palette): one channel of indices. Valid bit depths 1, 2, 4, 8.
    Indexed,
    /// Greyscale with alpha: two channels. Valid bit depths 8, 16.
    GrayscaleAlpha,
    /// Truecolour with alpha (RGBA): four channels. Valid bit depths 8, 16.
    TruecolorAlpha,
}

impl ColorType {
    /// The IHDR colour-type code (0, 2, 3, 4, or 6).
    #[must_use]
    pub fn code(self) -> u8 {
        match self {
            ColorType::Grayscale => 0,
            ColorType::Truecolor => 2,
            ColorType::Indexed => 3,
            ColorType::GrayscaleAlpha => 4,
            ColorType::TruecolorAlpha => 6,
        }
    }

    /// Channels (samples) per pixel.
    #[must_use]
    pub fn channels(self) -> usize {
        match self {
            ColorType::Grayscale | ColorType::Indexed => 1,
            ColorType::GrayscaleAlpha => 2,
            ColorType::Truecolor => 3,
            ColorType::TruecolorAlpha => 4,
        }
    }

    /// Whether `bit_depth` is permitted for this colour type (PNG Table 12).
    #[must_use]
    pub fn allows_bit_depth(self, bit_depth: u8) -> bool {
        match self {
            ColorType::Grayscale => matches!(bit_depth, 1 | 2 | 4 | 8 | 16),
            ColorType::Indexed => matches!(bit_depth, 1 | 2 | 4 | 8),
            ColorType::Truecolor | ColorType::GrayscaleAlpha | ColorType::TruecolorAlpha => {
                matches!(bit_depth, 8 | 16)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_channels() {
        assert_eq!(ColorType::Grayscale.code(), 0);
        assert_eq!(ColorType::Truecolor.code(), 2);
        assert_eq!(ColorType::Indexed.code(), 3);
        assert_eq!(ColorType::GrayscaleAlpha.code(), 4);
        assert_eq!(ColorType::TruecolorAlpha.code(), 6);
        assert_eq!(ColorType::Truecolor.channels(), 3);
        assert_eq!(ColorType::TruecolorAlpha.channels(), 4);
        assert_eq!(ColorType::GrayscaleAlpha.channels(), 2);
    }

    #[test]
    fn bit_depth_matrix() {
        assert!(ColorType::Grayscale.allows_bit_depth(1));
        assert!(ColorType::Grayscale.allows_bit_depth(16));
        assert!(!ColorType::Truecolor.allows_bit_depth(4));
        assert!(ColorType::Truecolor.allows_bit_depth(8));
        assert!(!ColorType::Indexed.allows_bit_depth(16));
        assert!(ColorType::Indexed.allows_bit_depth(4));
    }
}
