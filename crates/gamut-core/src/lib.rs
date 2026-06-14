//! Core traits, image buffers, dimensions, and error types shared across the gamut codecs.
//!
//! This crate is dependency-free with respect to the format crates: every codec in the
//! workspace builds on the [`EncodeImage`] / [`DecodeImage`] traits, the branded [`ImageRef`] /
//! [`ImageBuf`] buffers, and the [`Error`] type defined here, so that callers get a single,
//! consistent error surface regardless of format.
//!
//! # Scope and design notes
//!
//! The surface is deliberately small and frozen; the following are intentional decisions rather
//! than omissions:
//!
//! - **Interleaved `u8` / `u16` layouts only.** [`Sample`] is sealed over `u8` and `u16`; planar
//!   layouts and codec-side concerns such as coded bit depth live in `gamut-color`
//!   (`gamut_color::Planar8`, `gamut_color::BitDepth`), not here.
//! - **Open where growth is additive, sealed where it must not be.** [`Error`] and [`ColorModel`]
//!   are `#[non_exhaustive]` so variants can be added without a breaking change, while [`Pixel`]
//!   and [`Sample`] are sealed — the set of pixel layouts is closed and only this crate defines it.
//! - **Static error messages.** [`Error::InvalidInput`] / [`Error::Unsupported`] carry `&'static
//!   str`; dynamic context is deferred and can be added later as a new `#[non_exhaustive]` variant.
//! - **The length invariant lives on the buffers, not on [`Dimensions`].** [`Dimensions`] is a plain
//!   value type with public fields; non-emptiness and `len == width * height * channels` are
//!   enforced once, at [`ImageRef::new`] / [`ImageBuf::new`], so codecs receive a known-good buffer.
#![forbid(unsafe_code)]

mod image;
mod pixel;

pub use image::{ImageBuf, ImageRef};
pub use pixel::{
    Bilevel, Cmyk8, ColorModel, Gray8, Gray16, GrayAlpha8, GrayAlpha16, Indexed8, Pixel, Rgb8,
    Rgb16, Rgba8, Rgba16, Sample,
};

/// Errors produced by gamut encoders and decoders.
///
/// Marked `#[non_exhaustive]` so additional variants can be added as formats land without a
/// breaking change.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The input data was malformed, truncated, or otherwise not valid for the format.
    #[error("invalid input: {0}")]
    InvalidInput(&'static str),
    /// The requested format, profile, or feature is not yet supported.
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
}

/// Convenience result type for gamut operations.
pub type Result<T> = core::result::Result<T, Error>;

/// Width and height of an image, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Dimensions {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
}

impl Dimensions {
    /// Creates dimensions, rejecting a zero width or height.
    ///
    /// The fields stay public for ergonomic struct literals; this constructor is the validated
    /// path that buffer types ([`crate::ImageRef`]) and codecs use so an empty image is caught
    /// once, at the boundary, rather than re-checked in every encoder.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if either dimension is zero.
    pub fn new(width: u32, height: u32) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(Error::InvalidInput("zero-sized image"));
        }
        Ok(Self { width, height })
    }

    /// The pixel count `width * height`, or `None` if it overflows `usize`.
    #[must_use]
    pub fn num_pixels(self) -> Option<usize> {
        (self.width as usize).checked_mul(self.height as usize)
    }

    /// The sample count for an interleaved buffer of `channels` samples per pixel
    /// (`width * height * channels`), or `None` on overflow. The length an [`crate::ImageRef`]
    /// validates against.
    #[must_use]
    pub fn sample_count(self, channels: usize) -> Option<usize> {
        self.num_pixels()?.checked_mul(channels)
    }

    /// Whether either dimension is zero.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// Encodes an [`ImageRef`] of a specific pixel layout `P` into a compressed byte stream.
///
/// A codec implements this once per pixel layout it supports (`impl EncodeImage<Rgb8> for …`,
/// `impl EncodeImage<Cmyk8> for …`, …), so asking it to encode an unsupported layout is a compile
/// error rather than a runtime `Unsupported`. The input is pre-validated by [`ImageRef::new`], so an
/// implementation never re-checks the buffer length. Bytes are appended to `out` to keep callers
/// that reuse a scratch buffer allocation-conscious.
pub trait EncodeImage<P: Pixel> {
    /// Encode `image` into `out` (appended), returning the number of bytes written.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] if the requested encoder configuration is not implemented, or
    /// [`Error::InvalidInput`] if the image violates a format constraint (e.g. a dimension limit).
    fn encode_image(&self, image: ImageRef<'_, P>, out: &mut Vec<u8>) -> Result<usize>;

    /// Encode `image` into a fresh [`Vec`], returning the encoded bytes.
    ///
    /// A convenience over [`EncodeImage::encode_image`] for callers that just want the bytes;
    /// reach for `encode_image` with a reused `&mut Vec<u8>` when encoding many images and you want
    /// to amortise the allocation.
    ///
    /// # Errors
    ///
    /// As [`EncodeImage::encode_image`].
    fn encode_to_vec(&self, image: ImageRef<'_, P>) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        self.encode_image(image, &mut out)?;
        Ok(out)
    }
}

/// Decodes a compressed byte stream into an owned [`ImageBuf`] of pixel layout `P`.
///
/// `P` selects the layout the caller wants back; a codec implements this for each layout it can
/// present (converting internally as needed, e.g. grayscale → [`Rgb8`]). Returning an owned
/// [`ImageBuf`] keeps the dimensions, samples, and layout brand together so the result can't be
/// misinterpreted.
pub trait DecodeImage<P: Pixel> {
    /// Decode `data` into a fresh [`ImageBuf`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `data` is malformed, or [`Error::Unsupported`] if it uses
    /// a feature that is not implemented or cannot be presented as `P`.
    fn decode_image(&self, data: &[u8]) -> Result<ImageBuf<P>>;

    /// Decode `data` into `dst`, reusing its allocation when possible.
    ///
    /// The default implementation always replaces the buffer (`*dst = self.decode_image(data)?`). A
    /// codec may override it to reuse `dst`'s sample storage — via [`ImageBuf::as_mut_samples`] —
    /// across repeated calls whose decoded dimensions match `dst`'s, falling back to replacement
    /// otherwise. Either way `dst` holds the decoded image on success.
    ///
    /// # Errors
    ///
    /// As [`DecodeImage::decode_image`].
    fn decode_image_into(&self, data: &[u8], dst: &mut ImageBuf<P>) -> Result<()> {
        *dst = self.decode_image(data)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_displays_and_dimensions_fields() {
        assert!(!Error::Unsupported("x").to_string().is_empty());
        assert!(!Error::InvalidInput("y").to_string().is_empty());
        let d = Dimensions {
            width: 1920,
            height: 1080,
        };
        assert_eq!(d.width, 1920);
        assert_eq!(d.height, 1080);
    }

    #[test]
    fn dimensions_new_rejects_zero() {
        assert!(Dimensions::new(0, 4).is_err());
        assert!(Dimensions::new(4, 0).is_err());
        assert!(Dimensions::new(0, 0).is_err());
        let d = Dimensions::new(4, 3).unwrap();
        assert_eq!((d.width, d.height), (4, 3));
    }

    #[test]
    fn dimensions_pixel_and_sample_counts() {
        let d = Dimensions {
            width: 4,
            height: 3,
        };
        assert_eq!(d.num_pixels(), Some(12));
        assert_eq!(d.sample_count(3), Some(36));
        assert_eq!(d.sample_count(1), Some(12));
        assert!(!d.is_empty());
    }

    #[test]
    fn dimensions_is_empty() {
        assert!(
            Dimensions {
                width: 0,
                height: 5
            }
            .is_empty()
        );
        assert!(
            Dimensions {
                width: 5,
                height: 0
            }
            .is_empty()
        );
        assert!(
            !Dimensions {
                width: 5,
                height: 5
            }
            .is_empty()
        );
    }

    #[test]
    fn dimensions_sample_count_overflow_is_none() {
        // 65535*65535 fits in a 32-bit usize, so num_pixels is Some on every target...
        let d = Dimensions {
            width: 0xFFFF,
            height: 0xFFFF,
        };
        assert_eq!(d.num_pixels(), Some(0xFFFF * 0xFFFF));
        // ...but scaling by usize::MAX channels overflows on any platform.
        assert_eq!(d.sample_count(usize::MAX), None);
    }
}

#[cfg(test)]
mod trait_tests {
    use super::*;

    /// A trivial codec: encodes by copying the samples out, decodes a fixed 1x1 gray pixel.
    /// Exists only to exercise the trait defaults and object-safety.
    struct Trivial;

    impl EncodeImage<Gray8> for Trivial {
        fn encode_image(&self, image: ImageRef<'_, Gray8>, out: &mut Vec<u8>) -> Result<usize> {
            out.extend_from_slice(image.as_samples());
            Ok(image.as_samples().len())
        }
    }

    impl DecodeImage<Gray8> for Trivial {
        fn decode_image(&self, _data: &[u8]) -> Result<ImageBuf<Gray8>> {
            ImageBuf::<Gray8>::new(vec![42u8], Dimensions::new(1, 1)?)
        }
    }

    /// A codec that always fails, to exercise error propagation through provided methods.
    struct Failing;

    impl EncodeImage<Gray8> for Failing {
        fn encode_image(&self, _image: ImageRef<'_, Gray8>, _out: &mut Vec<u8>) -> Result<usize> {
            Err(Error::Unsupported("nope"))
        }
    }

    #[test]
    fn encode_image_appends_and_counts() {
        let img = ImageBuf::<Gray8>::new(vec![1, 2, 3, 4], Dimensions::new(2, 2).unwrap()).unwrap();
        let mut out = vec![0xFF];
        let n = Trivial.encode_image(img.as_ref(), &mut out).unwrap();
        assert_eq!(n, 4);
        assert_eq!(out, vec![0xFF, 1, 2, 3, 4]);
    }

    #[test]
    fn encode_to_vec_returns_fresh_exact_bytes() {
        let img = ImageBuf::<Gray8>::new(vec![1, 2, 3, 4], Dimensions::new(2, 2).unwrap()).unwrap();
        // A fresh Vec holding exactly the encoded bytes — no leading scratch, unlike encode_image
        // which appends. Asserting exact contents kills an "Ok(Vec::new())" mutant.
        assert_eq!(
            Trivial.encode_to_vec(img.as_ref()).unwrap(),
            vec![1, 2, 3, 4]
        );
    }

    #[test]
    fn encode_to_vec_propagates_errors() {
        let img = ImageBuf::<Gray8>::new(vec![0], Dimensions::new(1, 1).unwrap()).unwrap();
        // The default must surface encode_image's error rather than swallow it into an empty Vec.
        assert!(Failing.encode_to_vec(img.as_ref()).is_err());
    }

    #[test]
    fn decode_image_into_default_forwards() {
        let mut dst = ImageBuf::<Gray8>::zeroed(Dimensions::new(1, 1).unwrap()).unwrap();
        Trivial.decode_image_into(&[], &mut dst).unwrap();
        assert_eq!(dst.as_samples(), &[42]);
    }

    #[test]
    fn traits_are_object_safe() {
        // Compiles and runs only while both traits stay object-safe (e.g. for `Box<dyn …>`).
        let enc: &dyn EncodeImage<Gray8> = &Trivial;
        let dec: &dyn DecodeImage<Gray8> = &Trivial;
        let img = ImageBuf::<Gray8>::new(vec![7u8], Dimensions::new(1, 1).unwrap()).unwrap();
        let mut out = Vec::new();
        assert_eq!(enc.encode_image(img.as_ref(), &mut out).unwrap(), 1);
        assert_eq!(dec.decode_image(&[]).unwrap().as_samples(), &[42]);
    }
}
