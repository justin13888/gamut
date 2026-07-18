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
//!
//! # Limitations
//!
//! The crate codes the single still image. Some container features are deliberately deferred or out
//! of scope (see `STATUS.md` for the full matrix):
//!
//! - **Embedded metadata** — `ICCP` color profiles and `EXIF` / `XMP ` metadata are neither emitted
//!   on encode nor surfaced on decode (such chunks are skipped). Planned once the `gamut-metadata`
//!   facade lands (issue #34); the `gamut_core` still-image traits also carry no metadata channel yet.
//! - **Animation** — `ANIM` / `ANMF` multi-frame sequences are out of scope under the image-first
//!   charter. Each frame is an independent key frame, but assembling them needs a non-trait API.
//! - **Lossy quality** — the `0..=100` quality maps coarsely onto the VP8 base quantizer;
//!   rate-distortion tuning is tracked in issue #32.
//! - **Lossless** — always reproduces the input exactly and ignores the quality value; tuning
//!   compression density is tracked in issue #31.
//!
//! # Pluggable codestream backends
//!
//! The RIFF container and the coded picture are separable: [`backend`] exposes one trait pair —
//! [`WebpCodestreamDecoder`] / [`WebpCodestreamEncoder`], discriminated by [`WebpCodestream`] — that
//! routes a raw `VP8 ` / `VP8L` chunk payload to a hardware or alternate software codec, installed
//! with [`WebpDecoder::push_backend`] / [`WebpEncoder::push_backend`]. The crate's own `vp8`/`vp8l`
//! implementations are the implicit tails, so the default behaviour is unchanged. Backends written
//! against the shared [`gamut_codec_abi`] seam (issue #241) plug in through [`AbiDecoderBackend`] /
//! [`AbiEncoderBackend`].
#![forbid(unsafe_code)]

mod config;
mod decoder;
mod encoder;

pub mod alpha;
pub mod backend;
pub mod vp8;
pub mod vp8l;

pub use backend::{
    AbiDecoderBackend, AbiEncoderBackend, CodestreamInfo, DecodedRaster, PIXEL_FORMAT_ARGB,
    PIXEL_FORMAT_YUV420, RasterRef, WebpCodestream, WebpCodestreamDecoder, WebpCodestreamEncoder,
    WebpEncodeRequest,
};
pub use config::{WebpConfig, WebpMode};
pub use decoder::WebpDecoder;
pub use encoder::WebpEncoder;
pub use gamut_core::Dimensions;
