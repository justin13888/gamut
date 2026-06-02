//! AVIF (AV1 Image File Format) encoder — AV1 intra-frame bitstreams wrapped in an ISOBMFF/MIAF
//! container.
//!
//! M0 encodes a single still image **losslessly**: 8-bit RGB in, mapped to AV1 identity-matrix
//! 4:4:4 planes (so decoded output is bit-exact to the input), wrapped as one `av01` item. See
//! [`AvifEncoder`]. The roadmap (lossy, alpha, HDR, sequences, …) lives in `STATUS.md`.
//!
//! This crate is orchestration only: [`gamut_color`] maps pixels to planes, [`gamut_av1`] encodes
//! the AV1 temporal unit, and [`gamut_isobmff`] writes the container.
#![forbid(unsafe_code)]

mod encoder;

pub use encoder::AvifEncoder;
