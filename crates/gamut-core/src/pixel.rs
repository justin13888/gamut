//! Compile-time pixel vocabulary: the [`Sample`] storage primitives, the [`ColorModel`] tag, and
//! the sealed [`Pixel`] trait with one zero-sized marker per supported interleaved layout.
//!
//! These types brand the otherwise-opaque sample buffers in [`crate::ImageRef`] / [`crate::ImageBuf`]
//! so a layout mismatch — handing CMYK bytes to an RGBA encoder, or grayscale luminance where
//! palette indices are expected — is a compile error rather than a runtime length check. The markers
//! carry no data; they exist only to select an encoder/decoder impl and to expose the layout
//! constants ([`Pixel::CHANNELS`], [`Pixel::MODEL`], [`Pixel::BYTES_PER_PIXEL`]).

mod sample_sealed {
    pub trait Sealed {}
    impl Sealed for u8 {}
    impl Sealed for u16 {}
}

/// A pixel-sample storage primitive: `u8` (8-bit) or `u16` (10/12/16-bit, high-bit-depth).
///
/// Sealed — only `u8` and `u16` implement it. The supertrait bounds are chosen so that `P::Sample`
/// transitively gives buffer types everything they need (copy, zero-fill via `Default`, ordering)
/// without callers repeating `where P::Sample: …` clauses.
pub trait Sample:
    sample_sealed::Sealed + Copy + Default + Ord + core::fmt::Debug + 'static
{
    /// Bits the primitive stores (8 or 16). Distinct from a stream's *coded* bit depth (e.g. 10 or
    /// 12), which is a codec concern carried separately (see `gamut_color::BitDepth`).
    const STORAGE_BITS: u32;
}

impl Sample for u8 {
    const STORAGE_BITS: u32 = 8;
}
impl Sample for u16 {
    const STORAGE_BITS: u32 = 16;
}

/// The colour interpretation of a pixel's channels.
///
/// Distinguishes layouts that share a channel count: [`ColorModel::Rgba`] and [`ColorModel::Cmyk`]
/// are both four channels but must never be interchanged, and [`ColorModel::Gray`],
/// [`ColorModel::Bilevel`], and [`ColorModel::Indexed`] are all one channel with different meanings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ColorModel {
    /// Single luminance channel.
    Gray,
    /// Luminance plus an alpha channel.
    GrayAlpha,
    /// Red, green, blue.
    Rgb,
    /// Red, green, blue, alpha (unassociated).
    Rgba,
    /// Cyan, magenta, yellow, black ink separations.
    Cmyk,
    /// One channel, `0` = black and any non-zero value = white (a 1-bit image carried as one byte
    /// per pixel).
    Bilevel,
    /// One channel of indices into a separate colour palette.
    Indexed,
}

mod pixel_sealed {
    pub trait Sealed {}
}

/// Compile-time description of one interleaved pixel layout.
///
/// Sealed: implemented only by the zero-sized marker types in this module. A buffer is branded with
/// a `Pixel` type so its channel count, sample primitive, and colour model are known statically;
/// codecs implement [`crate::EncodeImage<P>`] / [`crate::DecodeImage<P>`] for exactly the `P` they
/// support, making an unsupported format a compile error.
pub trait Pixel: pixel_sealed::Sealed + Copy + 'static {
    /// The storage primitive of each sample (`u8` or `u16`).
    type Sample: Sample;
    /// Samples per pixel.
    const CHANNELS: usize;
    /// The colour interpretation of those samples.
    const MODEL: ColorModel;
    /// Bytes one pixel occupies in an interleaved buffer (`CHANNELS * size_of::<Sample>()`).
    const BYTES_PER_PIXEL: usize = Self::CHANNELS * core::mem::size_of::<Self::Sample>();
}

/// Defines a zero-sized pixel marker and its [`Pixel`] impl from a compact table.
macro_rules! define_pixels {
    ($(
        $(#[$meta:meta])*
        $name:ident => $sample:ty, $channels:expr, $model:expr;
    )*) => {
        $(
            $(#[$meta])*
            #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
            pub struct $name;

            impl pixel_sealed::Sealed for $name {}
            impl Pixel for $name {
                type Sample = $sample;
                const CHANNELS: usize = $channels;
                const MODEL: ColorModel = $model;
            }
        )*
    };
}

define_pixels! {
    /// 8-bit grayscale: one luminance byte per pixel.
    Gray8 => u8, 1, ColorModel::Gray;
    /// 8-bit bilevel: one byte per pixel, `0` = black and non-zero = white. Distinct from [`Gray8`]
    /// so a grayscale buffer cannot be mistaken for a 1-bit image.
    Bilevel => u8, 1, ColorModel::Bilevel;
    /// 8-bit palette indices: one index byte per pixel into a separate colour table. Distinct from
    /// [`Gray8`] so indices cannot be mistaken for luminance.
    Indexed8 => u8, 1, ColorModel::Indexed;
    /// 8-bit RGB: three interleaved bytes per pixel, row-major.
    Rgb8 => u8, 3, ColorModel::Rgb;
    /// 8-bit RGBA: four interleaved bytes per pixel (unassociated alpha).
    Rgba8 => u8, 4, ColorModel::Rgba;
    /// 8-bit CMYK: four interleaved ink bytes per pixel. Distinct from [`Rgba8`] despite the shared
    /// channel count.
    Cmyk8 => u8, 4, ColorModel::Cmyk;
    /// 8-bit grayscale + alpha: two interleaved bytes per pixel (luminance, then unassociated
    /// alpha). The PNG "greyscale with alpha" colour type.
    GrayAlpha8 => u8, 2, ColorModel::GrayAlpha;
    /// 16-bit grayscale: one `u16` luminance sample per pixel (high-bit-depth).
    Gray16 => u16, 1, ColorModel::Gray;
    /// 16-bit RGB: three interleaved `u16` samples per pixel (high-bit-depth).
    Rgb16 => u16, 3, ColorModel::Rgb;
    /// 16-bit RGBA: four interleaved `u16` samples per pixel (high-bit-depth).
    Rgba16 => u16, 4, ColorModel::Rgba;
    /// 16-bit grayscale + alpha: two interleaved `u16` samples per pixel (luminance, then
    /// unassociated alpha; high-bit-depth).
    GrayAlpha16 => u16, 2, ColorModel::GrayAlpha;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_storage_bits() {
        assert_eq!(<u8 as Sample>::STORAGE_BITS, 8);
        assert_eq!(<u16 as Sample>::STORAGE_BITS, 16);
    }

    #[test]
    fn pixel_layout_constants() {
        assert_eq!((Gray8::CHANNELS, Gray8::BYTES_PER_PIXEL), (1, 1));
        assert_eq!(Gray8::MODEL, ColorModel::Gray);
        assert_eq!((Bilevel::CHANNELS, Bilevel::BYTES_PER_PIXEL), (1, 1));
        assert_eq!(Bilevel::MODEL, ColorModel::Bilevel);
        assert_eq!((Indexed8::CHANNELS, Indexed8::BYTES_PER_PIXEL), (1, 1));
        assert_eq!(Indexed8::MODEL, ColorModel::Indexed);
        assert_eq!((Rgb8::CHANNELS, Rgb8::BYTES_PER_PIXEL), (3, 3));
        assert_eq!(Rgb8::MODEL, ColorModel::Rgb);
        assert_eq!((Rgba8::CHANNELS, Rgba8::BYTES_PER_PIXEL), (4, 4));
        assert_eq!(Rgba8::MODEL, ColorModel::Rgba);
        assert_eq!((Cmyk8::CHANNELS, Cmyk8::BYTES_PER_PIXEL), (4, 4));
        assert_eq!(Cmyk8::MODEL, ColorModel::Cmyk);
        assert_eq!((GrayAlpha8::CHANNELS, GrayAlpha8::BYTES_PER_PIXEL), (2, 2));
        assert_eq!(GrayAlpha8::MODEL, ColorModel::GrayAlpha);
        assert_eq!((Gray16::CHANNELS, Gray16::BYTES_PER_PIXEL), (1, 2));
        assert_eq!((Rgb16::CHANNELS, Rgb16::BYTES_PER_PIXEL), (3, 6));
        assert_eq!((Rgba16::CHANNELS, Rgba16::BYTES_PER_PIXEL), (4, 8));
        assert_eq!(
            (GrayAlpha16::CHANNELS, GrayAlpha16::BYTES_PER_PIXEL),
            (2, 4)
        );
        assert_eq!(GrayAlpha16::MODEL, ColorModel::GrayAlpha);
    }

    #[test]
    fn distinct_models_share_channel_count() {
        // Exactly the footgun the type system now prevents: same shape, different meaning.
        assert_eq!(Rgba8::CHANNELS, Cmyk8::CHANNELS);
        assert_ne!(Rgba8::MODEL, Cmyk8::MODEL);
        assert_eq!(Gray8::CHANNELS, Indexed8::CHANNELS);
        assert_ne!(Gray8::MODEL, Indexed8::MODEL);
        assert_ne!(Gray8::MODEL, Bilevel::MODEL);
    }
}
