//! WebP image encoder and decoder — an intra-frame VP8/VP8L still-image bitstream wrapped in a
//! RIFF container.
//!
//! The public surface mirrors [`gamut-avif`](https://docs.rs/gamut-avif): a [`WebpEncoder`]
//! implementing [`gamut_core::EncodeImage`] and a [`WebpDecoder`] implementing
//! [`gamut_core::DecodeImage`].
//! The container layer is [`gamut_riff`]; the codec layer is the [`vp8l`] (lossless, RFC 9649 §3)
//! and [`vp8`] (lossy intra, RFC 6386) module trees, whose modules each cite the spec section they
//! implement. The implementation status and milestones are tracked in `STATUS.md`.
//!
//! gamut is image-first, so only the intra/key-frame still-image subset of VP8 is in scope (no
//! inter-frame prediction, motion, or sequences). Both codecs are fully implemented, for
//! [`Rgb8`](gamut_core::Rgb8) and [`Rgba8`](gamut_core::Rgba8) input: **VP8L lossless**
//! (every transform, LZ77, the color cache, meta prefix codes) and **VP8 lossy** key-frame intra
//! (DC/V/H/TM and per-4×4 B_PRED prediction, the simple and normal loop filters, segmentation, 1/2/4/8
//! token partitions, and skip). Transparent lossy images use the extended (`VP8X`) container with an
//! `ALPH` alpha chunk. Every component is validated against libwebp as an oracle in both directions
//! (bit-exact at the YUV-plane level for lossy), plus a malformed-input robustness corpus.
#![forbid(unsafe_code)]

mod config;
mod decoder;
mod encoder;

pub mod alpha;
pub mod vp8;
pub mod vp8l;

pub use config::{WebpConfig, WebpMode};
pub use decoder::WebpDecoder;
pub use encoder::WebpEncoder;
pub use gamut_core::Dimensions;
