//! The [`JxlEncoder`]: a typed front end over the reference libjxl encoder ([`crate::ffi`]).
//!
//! Construct one with a mode — [`JxlEncoder::lossless`] (the default) or [`JxlEncoder::lossy`] — then
//! refine it with the chainable [`JxlEncoder::with_effort`] / [`JxlEncoder::with_container`] builders,
//! and drive it through the [`EncodeImage`] trait for any of the eight supported pixel layouts.

use gamut_core::{
    EncodeImage, Gray8, Gray16, GrayAlpha8, GrayAlpha16, ImageRef, Result, Rgb8, Rgb16, Rgba8,
    Rgba16,
};

use crate::config::{ColorSpec, Container, Distance, Effort, Mode, Orientation};
use crate::ffi::{self, FrameSpec, Samples};

/// A JPEG XL encoder backed by the reference libjxl.
///
/// Encodes 8- and 16-bit grayscale, gray+alpha, RGB and RGBA images. Pick a mode at construction —
/// [`JxlEncoder::lossless`] (bit-exact; also [`JxlEncoder::new`] and [`Default`]) or
/// [`JxlEncoder::lossy`] with a Butteraugli [`Distance`] — then optionally set the [`Effort`] and
/// output [`Container`] with the `with_*` builders. Encode through the
/// [`EncodeImage`](gamut_core::EncodeImage) trait, which appends the JPEG XL stream to the caller's
/// buffer.
///
/// The type is `Clone` but deliberately not `Copy`: a [`ColorSpec::Icc`] configuration owns the
/// profile bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct JxlEncoder {
    /// Lossless, or lossy at a validated distance.
    mode: Mode,
    /// The speed/density effort level.
    effort: Effort,
    /// Codestream vs. ISO BMFF container framing.
    container: Container,
    /// The colour interpretation signalled for the pixel samples.
    color: ColorSpec,
    /// The display orientation signalled for the coded samples.
    orientation: Orientation,
    /// Raw EXIF payload for an `Exif` container box, if any.
    exif: Option<Vec<u8>>,
    /// XMP (XML) payload for an `xml ` container box, if any.
    xmp: Option<Vec<u8>>,
}

impl Default for JxlEncoder {
    /// The default encoder is **lossless** — identical to [`JxlEncoder::lossless`], which (with
    /// [`JxlEncoder::new`]) is an intent-revealing alias for this canonical construction.
    fn default() -> Self {
        Self {
            mode: Mode::Lossless,
            effort: Effort::default(),
            container: Container::default(),
            color: ColorSpec::default(),
            orientation: Orientation::default(),
            exif: None,
            xmp: None,
        }
    }
}

impl JxlEncoder {
    /// Creates an encoder with the default configuration; equivalent to [`JxlEncoder::lossless`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an encoder that produces a **lossless** stream — the decoded image is bit-exact to the
    /// input. This is the default mode, so [`JxlEncoder::new`] and [`JxlEncoder::default`] return the
    /// same encoder; it exists to pair with [`JxlEncoder::lossy`] and make intent explicit.
    #[must_use]
    pub fn lossless() -> Self {
        Self::default()
    }

    /// Creates an encoder that produces a **lossy** stream at the given Butteraugli [`Distance`]
    /// (`1.0` = visually lossless; larger = smaller file, lower quality).
    #[must_use]
    pub fn lossy(distance: Distance) -> Self {
        Self {
            mode: Mode::Lossy(distance),
            ..Self::default()
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

    /// Sets the [`ColorSpec`] signalled for the pixel samples (sRGB by default). Returns the
    /// updated encoder for chaining.
    ///
    /// The encoder never converts pixels between colour spaces — this only declares how the
    /// caller's samples are to be interpreted. An [`ColorSpec::Icc`] profile is validated against
    /// the image's colour family when encoding.
    #[must_use]
    pub fn with_color(mut self, color: ColorSpec) -> Self {
        self.color = color;
        self
    }

    /// Sets the display [`Orientation`] signalled for the coded samples ([`Orientation::Identity`]
    /// by default). Returns the updated encoder for chaining.
    ///
    /// Orientation is metadata: the samples are stored exactly as given, and decoders apply the
    /// transform on output (the transposing variants swap the displayed width and height).
    #[must_use]
    pub fn with_orientation(mut self, orientation: Orientation) -> Self {
        self.orientation = orientation;
        self
    }

    /// Attaches a raw EXIF payload, stored as an `Exif` box in the ISO BMFF container. Returns the
    /// updated encoder for chaining.
    ///
    /// `exif` is the TIFF-structured EXIF data itself (starting with the `II`/`MM` byte-order
    /// mark); the 4-byte tiff-header offset the `Exif` box format requires is prepended
    /// automatically. Because metadata lives in container boxes, encoding requires
    /// [`Container::IsoBmff`] — combining metadata with [`Container::Codestream`] is a typed error
    /// rather than a silent framing change.
    #[must_use]
    pub fn with_exif(mut self, exif: &[u8]) -> Self {
        self.exif = Some(exif.to_vec());
        self
    }

    /// Attaches an XMP (XML) packet, stored as an `xml ` box in the ISO BMFF container. Returns
    /// the updated encoder for chaining.
    ///
    /// As with [`JxlEncoder::with_exif`], encoding then requires [`Container::IsoBmff`].
    #[must_use]
    pub fn with_xmp(mut self, xmp: &str) -> Self {
        self.xmp = Some(xmp.as_bytes().to_vec());
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

    /// The configured [`ColorSpec`].
    #[must_use]
    pub fn color(&self) -> &ColorSpec {
        &self.color
    }

    /// The configured [`Orientation`].
    #[must_use]
    pub fn orientation(&self) -> Orientation {
        self.orientation
    }

    /// The attached raw EXIF payload, if any.
    #[must_use]
    pub fn exif(&self) -> Option<&[u8]> {
        self.exif.as_deref()
    }

    /// The attached XMP packet bytes, if any.
    #[must_use]
    pub fn xmp(&self) -> Option<&[u8]> {
        self.xmp.as_deref()
    }

    /// The internal lossless/lossy mode, for the FFI driver.
    pub(crate) fn mode(&self) -> Mode {
        self.mode
    }

    /// Losslessly transcodes an existing JPEG bitstream into JPEG XL (the "jbrd" recompression
    /// path), appending to `out` and returning the number of bytes written.
    ///
    /// libjxl reversibly re-packs the JPEG's DCT coefficients and stores reconstruction metadata
    /// (the `jbrd` box), so the original `.jpg` can later be reconstructed bit-for-bit while the
    /// stream also decodes as ordinary JPEG XL pixels. Because that metadata lives in a container
    /// box, the output is **always ISO BMFF container framing** — the configured [`Container`] does
    /// not apply here. The transcode is inherently lossless, so the lossless/lossy mode and
    /// [`Distance`] do not apply either; only the configured [`Effort`] is honoured. Metadata
    /// attached with [`JxlEncoder::with_exif`]/[`JxlEncoder::with_xmp`] is **not** applied on this
    /// path: libjxl already carries the JPEG's own EXIF/XMP into the container automatically, and
    /// duplicating those boxes would corrupt the reconstruction metadata's byte accounting.
    ///
    /// # Errors
    ///
    /// Returns [`gamut_core::Error::InvalidInput`] on an empty or malformed JPEG codestream, and
    /// [`gamut_core::Error::Unsupported`] if the JPEG uses features whose reconstruction metadata
    /// libjxl cannot represent.
    pub fn recompress_jpeg(&self, jpeg: &[u8], out: &mut Vec<u8>) -> Result<usize> {
        ffi::recompress(self, jpeg, out)
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
    fn recompress_jpeg_rejects_empty_input() {
        let mut out = Vec::new();
        let err = JxlEncoder::new()
            .recompress_jpeg(&[], &mut out)
            .unwrap_err();
        assert!(matches!(
            err,
            gamut_core::Error::InvalidInput("JXL: empty JPEG input")
        ));
        // No output was produced on the rejected path.
        assert!(out.is_empty());
    }
}
