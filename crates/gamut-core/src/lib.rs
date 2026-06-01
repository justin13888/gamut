//! Core traits, image buffers, dimensions, and error types shared across the gamut codecs.
//!
//! This crate is dependency-free with respect to the format crates: every codec in the
//! workspace builds on the [`Encoder`] / [`Decoder`] traits and the [`Error`] type defined
//! here, so that callers get a single, consistent error surface regardless of format.
#![forbid(unsafe_code)]

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
}
