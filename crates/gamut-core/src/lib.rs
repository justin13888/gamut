//! Core traits, image buffers, dimensions, and error types shared across the gamut codecs.
//!
//! This crate is dependency-free with respect to the format crates: every codec in the
//! workspace builds on the [`Encoder`] / [`Decoder`] traits and the [`Error`] type defined
//! here, so that callers get a single, consistent error surface regardless of format.
#![forbid(unsafe_code)]

mod image;
mod pixel;

pub use image::{ImageBuf, ImageRef};
pub use pixel::{
    Bilevel, Cmyk8, ColorModel, Gray8, Gray16, Indexed8, Pixel, Rgb8, Rgb16, Rgba8, Rgba16, Sample,
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

/// Encodes an in-memory image into a compressed byte stream.
///
/// Implementations append the encoded bytes to `out` rather than allocating a fresh buffer,
/// keeping hot paths allocation-conscious for callers that reuse a scratch buffer.
pub trait Encoder {
    /// Encode `pixels` (described by `dims`) into `out`, returning the number of bytes written.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `pixels` does not match `dims`, or
    /// [`Error::Unsupported`] if the requested configuration is not implemented.
    fn encode(&self, pixels: &[u8], dims: Dimensions, out: &mut Vec<u8>) -> Result<usize>;
}

/// Decodes a compressed byte stream into raw pixels.
pub trait Decoder {
    /// Decode `data` into `out`, returning the decoded image [`Dimensions`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `data` is malformed, or [`Error::Unsupported`] if
    /// the stream uses a feature that is not implemented.
    fn decode(&self, data: &[u8], out: &mut Vec<u8>) -> Result<Dimensions>;
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
