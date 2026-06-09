//! AVIF (AV1 Image File Format) encoder — AV1 intra-frame bitstreams wrapped in an ISOBMFF/MIAF
//! container.
//!
//! Encodes a single still image: 8-bit RGB in, mapped to AV1 identity-matrix 4:4:4 planes and
//! wrapped as one `av01` item. **Lossless** by default (decoded output is bit-exact to the input);
//! a lossy quantizer is selected with [`AvifEncoder::with_qindex`]. See [`AvifEncoder`]. The
//! roadmap (more pixel formats, alpha, HDR, sequences, …) lives in `STATUS.md`.
//!
//! This crate is orchestration only: [`gamut_color`] maps pixels to planes, [`gamut_av1`] encodes
//! the AV1 temporal unit, and [`gamut_isobmff`] writes the container.
#![forbid(unsafe_code)]

mod encoder;

pub use encoder::AvifEncoder;
