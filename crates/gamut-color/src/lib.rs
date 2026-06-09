//! Color primitives for the gamut codecs: pixel formats, bit depths, chroma subsampling, the
//! CICP code points carried in nclx / AV1 sequence headers, and planar buffers.
//!
//! The M0 AVIF encoder uses only a narrow slice — 8-bit RGB in, mapped to identity (`mc = 0`)
//! 4:4:4 planes. The enums here intentionally model the wider spec surface (more formats, bit
//! depths, subsamplings, and CICP code points) so later milestones (M2 pixel formats, M4 HDR;
//! see `gamut-avif/STATUS.md`) extend without reshaping the types.
#![forbid(unsafe_code)]

pub mod cicp;
mod format;
mod planar;
mod ycbcr;

pub use cicp::{ColorRange, ColourPrimaries, MatrixCoefficients, TransferCharacteristics};
pub use format::{BitDepth, ChromaSubsampling, PixelFormat};
pub use planar::Planar8;
pub use ycbcr::{Yuv420, rgb_to_ycbcr, ycbcr_to_rgb};
