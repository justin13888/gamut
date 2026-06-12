//! Coded-plane bit depth and chroma subsampling.
//!
//! These describe a codec's *coded* planes, distinct from an interleaved buffer's layout — that is
//! the [`Pixel`](gamut_core::Pixel) vocabulary (`Rgb8`, `Rgba8`, …) in `gamut-core`. [`BitDepth`] is
//! wired into the AV1 reconstruction; [`ChromaSubsampling`] models only `Cs444` (4:4:4) at M0, with
//! the subsampled variants reserved for M2 (see `gamut-avif/STATUS.md`).

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

    /// The [`BitDepth`] for `bits` (8, 10, or 12), or `None` for any other value. The inverse of
    /// [`BitDepth::bits`], for turning a codec's raw integer bit depth back into the typed form.
    #[must_use]
    pub fn from_bits(bits: u32) -> Option<Self> {
        match bits {
            8 => Some(BitDepth::Eight),
            10 => Some(BitDepth::Ten),
            12 => Some(BitDepth::Twelve),
            _ => None,
        }
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
    fn bit_depth_bits() {
        assert_eq!(BitDepth::Eight.bits(), 8);
        assert_eq!(BitDepth::Ten.bits(), 10);
        assert_eq!(BitDepth::Twelve.bits(), 12);
        // from_bits is the inverse of bits() for the three valid depths; other values are None.
        for d in [BitDepth::Eight, BitDepth::Ten, BitDepth::Twelve] {
            assert_eq!(BitDepth::from_bits(u32::from(d.bits())), Some(d));
        }
        assert_eq!(BitDepth::from_bits(16), None);
        assert_eq!(BitDepth::from_bits(0), None);
    }

    #[test]
    fn subsampling_flags() {
        assert_eq!(ChromaSubsampling::Cs444.subsampling(), (0, 0));
        assert_eq!(ChromaSubsampling::Cs420.subsampling(), (1, 1));
        assert_eq!(ChromaSubsampling::Cs422.subsampling(), (1, 0));
    }
}
