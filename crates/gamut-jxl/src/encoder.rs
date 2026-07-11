//! The [`JxlEncoder`]: a typed front end over the reference libjxl encoder ([`crate::ffi`]).
//!
//! Construct one with a mode — [`JxlEncoder::lossless`] (the default) or [`JxlEncoder::lossy`] — then
//! refine it with the chainable [`JxlEncoder::with_effort`] / [`JxlEncoder::with_container`] builders,
//! and drive it through the [`EncodeImage`] trait for any of the eight supported pixel layouts.

use gamut_core::{
    EncodeImage, Gray8, Gray16, GrayAlpha8, GrayAlpha16, ImageRef, Result, Rgb8, Rgb16, Rgba8,
    Rgba16,
};

use crate::config::{Container, Distance, Effort, Mode};
use crate::ffi::{self, FrameSpec, Samples};

/// A JPEG XL encoder backed by the reference libjxl.
///
/// Encodes 8- and 16-bit grayscale, gray+alpha, RGB and RGBA images. Pick a mode at construction —
/// [`JxlEncoder::lossless`] (bit-exact; also [`JxlEncoder::new`] and [`Default`]) or
/// [`JxlEncoder::lossy`] with a Butteraugli [`Distance`] — then optionally set the [`Effort`] and
/// output [`Container`] with the `with_*` builders. Encode through the
/// [`EncodeImage`](gamut_core::EncodeImage) trait, which appends the JPEG XL stream to the caller's
/// buffer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JxlEncoder {
    /// Lossless, or lossy at a validated distance.
    mode: Mode,
    /// The speed/density effort level.
    effort: Effort,
    /// Codestream vs. ISO BMFF container framing.
    container: Container,
}

impl Default for JxlEncoder {
    /// The default encoder is **lossless** — identical to [`JxlEncoder::lossless`].
    fn default() -> Self {
        Self::lossless()
    }
}

impl JxlEncoder {
    /// Creates an encoder with the default configuration; equivalent to [`JxlEncoder::lossless`].
    #[must_use]
    pub fn new() -> Self {
        Self::lossless()
    }

    /// Creates an encoder that produces a **lossless** stream — the decoded image is bit-exact to the
    /// input. This is the default mode, so [`JxlEncoder::new`] and [`JxlEncoder::default`] return the
    /// same encoder; it exists to pair with [`JxlEncoder::lossy`] and make intent explicit.
    #[must_use]
    pub fn lossless() -> Self {
        Self {
            mode: Mode::Lossless,
            effort: Effort::default(),
            container: Container::default(),
        }
    }

    /// Creates an encoder that produces a **lossy** stream at the given Butteraugli [`Distance`]
    /// (`1.0` = visually lossless; larger = smaller file, lower quality).
    #[must_use]
    pub fn lossy(distance: Distance) -> Self {
        Self {
            mode: Mode::Lossy(distance),
            effort: Effort::default(),
            container: Container::default(),
        }
    }

    /// Sets the [`Effort`] (speed/density trade-off). Returns the updated encoder for chaining.
    #[must_use]
    pub fn with_effort(mut self, effort: Effort) -> Self {
        self.effort = effort;
        self
    }

    /// Sets the output [`Container`] framing (bare codestream vs. ISO BMFF). Returns the updated
    /// encoder for chaining.
    #[must_use]
    pub fn with_container(mut self, container: Container) -> Self {
        self.container = container;
        self
    }

    /// Whether this encoder is in lossless mode.
    #[must_use]
    pub fn is_lossless(&self) -> bool {
        matches!(self.mode, Mode::Lossless)
    }

    /// The configured lossy [`Distance`], or `None` in lossless mode.
    #[must_use]
    pub fn distance(&self) -> Option<Distance> {
        match self.mode {
            Mode::Lossless => None,
            Mode::Lossy(d) => Some(d),
        }
    }

    /// The configured [`Effort`].
    #[must_use]
    pub fn effort(&self) -> Effort {
        self.effort
    }

    /// The configured output [`Container`].
    #[must_use]
    pub fn container(&self) -> Container {
        self.container
    }

    /// The internal lossless/lossy mode, for the FFI driver.
    pub(crate) fn mode(&self) -> Mode {
        self.mode
    }

    /// Losslessly transcodes an existing JPEG bitstream into JPEG XL (the "jbrd" recompression path),
    /// appending to `out` and returning the number of bytes written.
    ///
    /// This is the API slot for libjxl's JPEG-reconstruction feature, which reversibly re-packs a
    /// baseline JPEG's coefficients so the original `.jpg` can be reconstructed bit-for-bit. It is
    /// **not yet implemented**; the method exists so the eventual encoder gains it without a breaking
    /// signature change. See `STATUS.md` for the deferral rationale.
    ///
    /// # Errors
    ///
    /// Always returns [`gamut_core::Error::Unsupported`] in this version.
    pub fn recompress_jpeg(&self, jpeg: &[u8], out: &mut Vec<u8>) -> Result<usize> {
        let _ = (jpeg, out);
        Err(gamut_core::Error::Unsupported(
            "JXL: JPEG bitstream recompression is not yet implemented",
        ))
    }
}

/// Implements [`EncodeImage`] for each supported pixel layout by building a [`FrameSpec`] and handing
/// it to the FFI driver. Every arm names its `bits_per_sample`, colour-channel count, alpha presence,
/// and sample variant so the frame description stays consistent with the layout brand.
macro_rules! impl_encode_image {
    ($($pixel:ty => $variant:ident, $bits:expr, $color_channels:expr, $has_alpha:expr;)*) => {$(
        impl EncodeImage<$pixel> for JxlEncoder {
            fn encode_image(&self, image: ImageRef<'_, $pixel>, out: &mut Vec<u8>) -> Result<usize> {
                let dims = image.dimensions();
                let spec = FrameSpec {
                    width: dims.width,
                    height: dims.height,
                    bits_per_sample: $bits,
                    num_color_channels: $color_channels,
                    has_alpha: $has_alpha,
                    samples: Samples::$variant(image.as_samples()),
                };
                ffi::encode(self, spec, out)
            }
        }
    )*};
}

impl_encode_image! {
    Gray8       => U8,  8,  1, false;
    GrayAlpha8  => U8,  8,  1, true;
    Rgb8        => U8,  8,  3, false;
    Rgba8       => U8,  8,  3, true;
    Gray16      => U16, 16, 1, false;
    GrayAlpha16 => U16, 16, 1, true;
    Rgb16       => U16, 16, 3, false;
    Rgba16      => U16, 16, 3, true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_and_new_are_lossless() {
        assert!(JxlEncoder::default().is_lossless());
        assert!(JxlEncoder::new().is_lossless());
        assert_eq!(JxlEncoder::new(), JxlEncoder::lossless());
        assert_eq!(JxlEncoder::default(), JxlEncoder::lossless());
        // A lossless encoder exposes no distance.
        assert_eq!(JxlEncoder::lossless().distance(), None);
    }

    #[test]
    fn defaults_are_squirrel_effort_and_codestream() {
        let enc = JxlEncoder::new();
        assert_eq!(enc.effort(), Effort::Squirrel);
        assert_eq!(enc.container(), Container::Codestream);
    }

    #[test]
    fn lossy_carries_its_distance_and_is_not_lossless() {
        let d = Distance::new(2.5).unwrap();
        let enc = JxlEncoder::lossy(d);
        assert!(!enc.is_lossless());
        assert_eq!(enc.distance(), Some(d));
    }

    #[test]
    fn builders_are_chainable_and_preserve_mode() {
        let enc = JxlEncoder::lossy(Distance::new(3.0).unwrap())
            .with_effort(Effort::Glacier)
            .with_container(Container::IsoBmff);
        assert_eq!(enc.effort(), Effort::Glacier);
        assert_eq!(enc.container(), Container::IsoBmff);
        // The builders touch only their own field, not the mode.
        assert!(!enc.is_lossless());
        assert_eq!(enc.distance(), Some(Distance::new(3.0).unwrap()));

        // with_effort on a lossless encoder keeps it lossless.
        let l = JxlEncoder::lossless().with_effort(Effort::Lightning);
        assert!(l.is_lossless());
        assert_eq!(l.effort(), Effort::Lightning);
    }

    #[test]
    fn recompress_jpeg_is_unsupported() {
        let mut out = Vec::new();
        let err = JxlEncoder::new()
            .recompress_jpeg(&[0xFF, 0xD8, 0xFF], &mut out)
            .unwrap_err();
        assert!(matches!(err, gamut_core::Error::Unsupported(_)));
        // No output was produced on the unsupported path.
        assert!(out.is_empty());
    }
}
