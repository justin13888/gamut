//! `gamut-png` — a research-grade, space-efficient **PNG encoder** (PNG 3rd edition; W3C).
//!
//! PNG is a lossless raster format: an 8-byte signature followed by typed chunks (IHDR, optional
//! palette/colour/metadata chunks, IDAT image data, IEND). The image data is scanline-filtered and
//! then DEFLATE-compressed. This crate builds on [`gamut_deflate`] for the compression and aims for
//! output sizes on par with the best PNG encoders, trading encode time for size at higher levels.
//!
//! # Encoder only
//!
//! Per gamut's encoder-first philosophy and issue #24, this crate **does not decode** — PNG decoding
//! is a thoroughly solved problem. Correctness is proven differentially: a vendored libpng decodes
//! the encoder's output and the pixels must match exactly.
//!
//! # Example
//!
//! ```
//! use gamut_core::{Dimensions, EncodeImage, ImageRef, Rgb8};
//! use gamut_png::PngEncoder;
//!
//! let (w, h) = (2, 2);
//! let rgb = vec![0u8; (w * h * 3) as usize];
//! let image = ImageRef::<Rgb8>::new(&rgb, Dimensions::new(w, h).unwrap()).unwrap();
//! let mut png = Vec::new();
//! PngEncoder::new().encode_image(image, &mut png).unwrap();
//! assert_eq!(&png[1..4], b"PNG");
//! ```
#![forbid(unsafe_code)]

mod ancillary;
mod chunk;
mod color;
mod crc32;
mod encoder;
mod filter;
mod ihdr;
mod pack;
mod palette;
mod reduce;

pub use ancillary::{PhysicalUnit, SrgbIntent};
pub use color::ColorType;
pub use encoder::PngEncoder;
pub use filter::{FilterStrategy, FilterType};
pub use palette::PngPalette;
