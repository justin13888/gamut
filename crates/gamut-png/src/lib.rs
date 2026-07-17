//! `gamut-png` — a research-grade, space-efficient **PNG codec** (PNG 3rd edition; W3C).
//!
//! PNG is a lossless raster format: an 8-byte signature followed by typed chunks (IHDR, optional
//! palette/colour/metadata chunks, IDAT image data, IEND). The image data is scanline-filtered and
//! then DEFLATE-compressed. The encoder builds on [`gamut_deflate`] for the compression and aims
//! for output sizes on par with the best PNG encoders, trading encode time for size at higher
//! levels. The decoder ([`PngDecoder`], issue #249) covers the full still-image spec — every
//! colour type and bit depth, Adam7 interlacing, all filters — behind hostile-input limits, and
//! surfaces ancillary metadata (EXIF/ICC/XMP/text) as raw payloads. Animation (APNG) is out of
//! scope. Correctness in both directions is proven differentially against a vendored libpng.
//!
//! # Example
//!
//! ```
//! use gamut_core::{DecodeImage, Dimensions, EncodeImage, ImageBuf, ImageRef, Rgb8};
//! use gamut_png::{PngDecoder, PngEncoder};
//!
//! let (w, h) = (2, 2);
//! let rgb = vec![7u8; (w * h * 3) as usize];
//! let image = ImageRef::<Rgb8>::new(&rgb, Dimensions::new(w, h).unwrap()).unwrap();
//! let mut png = Vec::new();
//! PngEncoder::new().encode_image(image, &mut png).unwrap();
//! assert_eq!(&png[1..4], b"PNG");
//!
//! let decoded: ImageBuf<Rgb8> = PngDecoder::new().decode_image(&png).unwrap();
//! assert_eq!(decoded.as_samples(), rgb);
//! ```
#![forbid(unsafe_code)]

mod ancillary;
mod chunk;
mod color;
mod crc32;
mod decoder;
mod encoder;
mod filter;
mod ihdr;
mod inflate;
mod pack;
mod palette;
mod reduce;

pub use ancillary::{PhysicalUnit, SrgbIntent};
pub use color::ColorType;
pub use decoder::{PngDecoder, TransparencyKey};
pub use encoder::PngEncoder;
pub use filter::{FilterStrategy, FilterType};
/// The DEFLATE compression level, accepted by [`PngEncoder::with_compression`].
pub use gamut_deflate::Level;
pub use palette::PngPalette;
