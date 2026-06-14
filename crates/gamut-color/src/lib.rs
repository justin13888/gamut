//! Color primitives for the gamut codecs: pixel formats, bit depths, chroma subsampling, the
//! CICP code points carried in nclx / AV1 sequence headers, and planar buffers.
//!
//! The M0 AVIF encoder uses only a narrow slice — 8-bit RGB in, mapped to identity (`mc = 0`)
//! 4:4:4 planes. The enums here intentionally model the wider spec surface (more formats, bit
//! depths, subsamplings, and CICP code points) so later milestones (M2 pixel formats, M4 HDR;
//! see `gamut-avif/STATUS.md`) extend without reshaping the types.
//!
//! On top of that metadata layer, the [`transfer`], [`oklab`], [`matrix`], [`gamut_map`], and
//! [`profile`] modules add `f64` colour science — encoder-exact EOTFs, OKLab transforms with
//! per-gamut matrices (derived from chromaticities via Bradford adaptation), gamut clamping, and
//! source-profile bundles over the CICP axes. This math is **Tier-1** (correctness only): it uses
//! `std` `f64`, so it is not bit-reproducible across platforms — see `references/color/README.md`.
#![forbid(unsafe_code)]

pub mod cicp;
mod format;
pub mod gamut_map;
mod linalg;
pub mod matrix;
pub mod oklab;
mod pixel;
mod planar;
pub mod profile;
pub mod transfer;
mod ycbcr;

pub use cicp::{ColorRange, ColourPrimaries, MatrixCoefficients, TransferCharacteristics};
pub use format::{BitDepth, ChromaSubsampling};
pub use pixel::{clip_pixel, clip_pixel8};
pub use planar::Planar8;
pub use ycbcr::{Yuv420, rgb_to_ycbcr, ycbcr_to_rgb};
