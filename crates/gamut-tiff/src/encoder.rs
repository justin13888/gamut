//! The TIFF encoder.

use crate::compression::Compression;
use crate::ifd::{PhotometricInterpretation, Predictor};

/// Encoder for TIFF 6.0 images.
///
/// Will implement [`gamut_core::Encoder`], writing an 8-byte header, one or more IFDs, and the
/// strip/tile image data. Implementation pending (see issue #107).
pub struct TiffEncoder;

/// Options controlling how a TIFF image is written.
pub struct TiffEncodeOptions {
    /// The compression scheme applied to the image data.
    pub compression: Compression,
    /// How the written samples map to colour.
    pub photometric: PhotometricInterpretation,
    /// The prediction applied before compression.
    pub predictor: Predictor,
}
