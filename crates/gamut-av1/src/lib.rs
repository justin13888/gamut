//! AV1 image encoder. AVIF relies on AV1 intra-frame coding, so this crate is available
//! standalone as well as through [`gamut-avif`](https://crates.io/crates/gamut-avif).
//!
//! M0 implemented the minimal path: a **lossless** all-intra keyframe — `seq_profile = 1`
//! (8-bit 4:4:4), identity matrix coefficients, full range, single tile, 64×64 superblocks,
//! `DC_PRED`, and the forced `TX_4X4` Walsh–Hadamard transform. The crate now also encodes
//! **lossy** all-intra keyframes: a wide intra mode set (the eight directional modes with
//! `angle_delta`, `SMOOTH`/`SMOOTH_V`/`SMOOTH_H`/`PAETH`, recursive filter-intra, chroma-from-luma,
//! and palette), DCT/ADST/identity transforms with variable `tx_depth` and `TX_SET_INTRA_2` type
//! selection, recursive partitioning, per-superblock delta-Q/delta-LF, segmentation, uniform
//! multi-tile, and the in-loop filters (deblocking, CDEF, loop restoration, superres) — still with
//! static default CDFs (`disable_cdf_update = 1`). It produces the AV1 temporal unit that
//! `gamut-avif` wraps in an AVIF still image. The remaining surface (10/12-bit, 4:2:0/4:2:2,
//! monochrome, CDF adaptation, quantizer matrices, and the AVIF-level alpha/metadata/container
//! features) is tracked in `gamut-avif/STATUS.md`.
//!
//! Modules mirror the spec: [`headers`] = OBU framing + sequence/frame headers (AV1 §5.3/§5.5/§5.9),
//! `tile` = partition/prediction/coefficient coding (§5.11), [`transform`] = forward/inverse 2-D
//! transforms (§7.13), `cdf` = default CDF + scan + context tables (§9.2/§9.4/§8.3.2),
//! [`quant`] = quantizer tables + dequant (§7.12), `filter` = in-loop filters (§7.14-§7.17).
#![forbid(unsafe_code)]

mod cdf;
mod encoder;
mod filter;
mod headers;
pub mod quant;
mod tile;
pub mod transform;

pub use encoder::{
    EncodedStill, ReconImage, encode_still_intra, encode_still_intra_superres,
    encode_still_lossless_identity,
};
pub use headers::Av1StillConfig;
