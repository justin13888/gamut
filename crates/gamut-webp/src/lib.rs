//! WebP image encoder and decoder — an intra-frame VP8/VP8L still-image bitstream wrapped in a
//! RIFF container.
//!
//! The public surface mirrors [`gamut-avif`](https://docs.rs/gamut-avif): a [`WebpEncoder`]
//! implementing [`gamut_core::Encoder`] and a [`WebpDecoder`] implementing [`gamut_core::Decoder`].
//! The container layer is [`gamut_riff`]; the codec layer is the [`vp8l`] (lossless, RFC 9649 §3)
//! and [`vp8`] (lossy intra, RFC 6386) module trees, whose modules each cite the spec section they
//! implement. The implementation status and milestones are tracked in `STATUS.md`.
//!
//! gamut is image-first, so only the intra/key-frame still-image subset of VP8 is in scope (no
//! inter-frame prediction, motion, or sequences). The bitstream paths are under construction:
//! [`WebpEncoder`] validates inputs then returns [`gamut_core::Error::Unsupported`] until the VP8L
//! M0 path lands, while [`WebpDecoder`] already parses the RIFF container and routes by bitstream
//! type. Every implemented component is validated against libwebp as an oracle (see the crate's
//! `tests/`).
#![forbid(unsafe_code)]

mod config;
mod decoder;
mod encoder;

pub mod vp8;
pub mod vp8l;

pub use config::{WebpConfig, WebpMode};
pub use decoder::WebpDecoder;
pub use encoder::WebpEncoder;
