//! Pixel formats, bit depths, and chroma subsampling.
//!
//! M0 uses [`PixelFormat::Rgb8`], [`BitDepth::Eight`], and [`ChromaSubsampling::Cs444`]; the other
//! variants model the spec surface for later milestones (see `gamut-avif/STATUS.md`).

/// Layout of an interleaved input pixel buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// 8-bit RGB, 3 interleaved bytes per pixel, row-major, no padding.
    Rgb8,
    /// 8-bit RGBA, 4 interleaved bytes per pixel (alpha handled at M3).
    Rgba8,
}

impl PixelFormat {
    /// Bytes per pixel in this layout.
    #[must_use]
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            PixelFormat::Rgb8 => 3,
            PixelFormat::Rgba8 => 4,
        }
    }
}

/// Bits per sample of a coded plane.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BitDepth {
    /// 8 bits per sample.
    Eight = 8,
    /// 10 bits per sample (M2).
    Ten = 10,
    /// 12 bits per sample (M2).
    Twelve = 12,
}

impl BitDepth {
    /// Number of bits per sample.
    #[must_use]
    pub fn bits(self) -> u8 {
        self as u8
    }
}

/// Chroma subsampling of the coded planes (AV1 `subsampling_x` / `subsampling_y`, §5.5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChromaSubsampling {
    /// 4:4:4 — full-resolution chroma (`subsampling_x = subsampling_y = 0`). Required for identity.
    Cs444,
    /// 4:2:2 — horizontally halved chroma (M2).
    Cs422,
    /// 4:2:0 — halved in both directions (M2).
    Cs420,
    /// 4:0:0 — monochrome, no chroma planes (M2).
    Cs400,
}

impl ChromaSubsampling {
    /// Returns `(subsampling_x, subsampling_y)` as the AV1 sequence-header flags.
    #[must_use]
    pub fn subsampling(self) -> (u8, u8) {
        match self {
            ChromaSubsampling::Cs444 | ChromaSubsampling::Cs400 => (0, 0),
            ChromaSubsampling::Cs422 => (1, 0),
            ChromaSubsampling::Cs420 => (1, 1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_format_bpp() {
        assert_eq!(PixelFormat::Rgb8.bytes_per_pixel(), 3);
        assert_eq!(PixelFormat::Rgba8.bytes_per_pixel(), 4);
    }

    #[test]
    fn bit_depth_bits() {
        assert_eq!(BitDepth::Eight.bits(), 8);
        assert_eq!(BitDepth::Ten.bits(), 10);
        assert_eq!(BitDepth::Twelve.bits(), 12);
    }

    #[test]
    fn subsampling_flags() {
        assert_eq!(ChromaSubsampling::Cs444.subsampling(), (0, 0));
        assert_eq!(ChromaSubsampling::Cs420.subsampling(), (1, 1));
        assert_eq!(ChromaSubsampling::Cs422.subsampling(), (1, 0));
    }
}
