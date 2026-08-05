//! Coded-plane bit depth and chroma subsampling.
//!
//! These describe a codec's *coded* planes, distinct from an interleaved buffer's layout — that is
//! the [`Pixel`](gamut_core::Pixel) vocabulary (`Rgb8`, `Rgba8`, …) in `gamut-core`. [`BitDepth`] is
//! wired into the AV1 reconstruction; [`ChromaSubsampling`] models only `Cs444` (4:4:4) at M0, with
//! the subsampled variants reserved for M2 (see `gamut-avif/STATUS.md`).

/// Bits per sample of a coded plane.
///
/// `#[non_exhaustive]`: models the AV1 profile depths plus the 16-bit depth of the wider
/// still-image pipelines today; other codecs may add depths later. Formats whose depth is
/// free-form rather than a fixed set — TIFF and DNG allow any of `1..=16` — carry it as a plain
/// integer instead (`gamut-dng`'s `bits_per_sample`), not as this enum.
#[repr(u8)]
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BitDepth {
    /// 8 bits per sample.
    Eight = 8,
    /// 10 bits per sample (M2).
    Ten = 10,
    /// 12 bits per sample (M2).
    Twelve = 12,
    /// 16 bits per sample. Not an AV1 profile depth — this is the depth of the 16-bit
    /// interleaved pipelines (PNG/TIFF/JXL/DNG) that share this vocabulary.
    Sixteen = 16,
}

impl BitDepth {
    /// Number of bits per sample.
    #[must_use]
    pub fn bits(self) -> u8 {
        self as u8
    }

    /// The largest sample value a plane of this depth can hold, `(1 << bits) - 1` (255, 1023,
    /// 4095, 65535) — the white level of a full-range plane, and the divisor for normalizing a
    /// sample to `0.0..=1.0`. Returns `u16` because every modeled depth fits one.
    #[must_use]
    pub fn max_value(self) -> u16 {
        // Shifting `u16::MAX` down is exact and total: `1u16 << 16` would overflow at `Sixteen`,
        // and `(1u32 << bits) - 1` would need a truncating cast back to `u16`.
        u16::MAX >> (16 - u32::from(self.bits()))
    }

    /// The [`BitDepth`] for `bits` (8, 10, 12, or 16), or `None` for any other value. The inverse
    /// of [`BitDepth::bits`], for turning a codec's raw integer bit depth back into the typed
    /// form. Takes `u32` because that is what codec headers and reconstruction paths carry;
    /// [`BitDepth::bits`] returns the exact `u8`.
    #[must_use]
    pub fn from_bits(bits: u32) -> Option<Self> {
        match bits {
            8 => Some(BitDepth::Eight),
            10 => Some(BitDepth::Ten),
            12 => Some(BitDepth::Twelve),
            16 => Some(BitDepth::Sixteen),
            _ => None,
        }
    }
}

/// Chroma subsampling of the coded planes (AV1 `subsampling_x` / `subsampling_y`, §5.5.2).
///
/// `#[non_exhaustive]`: models the AV1 layouts today; other codecs may add layouts (e.g. TIFF's
/// 4:1:1) later.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
    /// Returns `(subsampling_x, subsampling_y)` as the AV1 sequence-header flags. Monochrome is
    /// signalled by `mono_chrome`, which fixes both flags to 1 (AV1 §6.4.2), not by a subsampling
    /// combination of its own.
    #[must_use]
    pub fn subsampling(self) -> (u8, u8) {
        match self {
            ChromaSubsampling::Cs444 => (0, 0),
            ChromaSubsampling::Cs422 => (1, 0),
            ChromaSubsampling::Cs420 | ChromaSubsampling::Cs400 => (1, 1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [BitDepth; 4] = [
        BitDepth::Eight,
        BitDepth::Ten,
        BitDepth::Twelve,
        BitDepth::Sixteen,
    ];

    #[test]
    fn bit_depth_bits() {
        assert_eq!(BitDepth::Eight.bits(), 8);
        assert_eq!(BitDepth::Ten.bits(), 10);
        assert_eq!(BitDepth::Twelve.bits(), 12);
        assert_eq!(BitDepth::Sixteen.bits(), 16);
        // from_bits is the inverse of bits() for the four modeled depths; other values are None.
        for d in ALL {
            assert_eq!(BitDepth::from_bits(u32::from(d.bits())), Some(d));
        }
        assert_eq!(BitDepth::from_bits(0), None);
        // 14 is a real RAW sensor depth, deliberately not modeled: depths outside this fixed set
        // are free-form and carried as a plain integer by the format crate (see the enum's docs).
        assert_eq!(BitDepth::from_bits(14), None);
    }

    #[test]
    fn bit_depth_max_value() {
        assert_eq!(BitDepth::Eight.max_value(), 255);
        assert_eq!(BitDepth::Ten.max_value(), 1023);
        assert_eq!(BitDepth::Twelve.max_value(), 4095);
        assert_eq!(BitDepth::Sixteen.max_value(), 65535);
        // Cross-check against the (1 << bits) - 1 definition computed in u32 — a different
        // expression than max_value's own downshift of u16::MAX.
        for d in ALL {
            let expect = (1u32 << d.bits()) - 1;
            assert_eq!(u32::from(d.max_value()), expect);
        }
    }

    #[test]
    fn subsampling_flags() {
        assert_eq!(ChromaSubsampling::Cs444.subsampling(), (0, 0));
        assert_eq!(ChromaSubsampling::Cs420.subsampling(), (1, 1));
        assert_eq!(ChromaSubsampling::Cs422.subsampling(), (1, 0));
        // Monochrome carries subsampling_x = subsampling_y = 1 (AV1 §6.4.2).
        assert_eq!(ChromaSubsampling::Cs400.subsampling(), (1, 1));
    }
}
