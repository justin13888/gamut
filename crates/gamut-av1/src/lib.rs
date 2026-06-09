//! AV1 image encoder. AVIF relies on AV1 intra-frame coding, so this crate is available
//! standalone as well as through [`gamut-avif`](https://crates.io/crates/gamut-avif).
//!
//! M0 implements a single, narrow path: a **lossless** all-intra keyframe — `seq_profile = 1`
//! (8-bit 4:4:4), identity matrix coefficients, full range, single tile, 64×64 superblocks,
//! `DC_PRED`, and the forced `TX_4X4` Walsh–Hadamard transform, with static default CDFs
//! (`disable_cdf_update = 1`). It produces the AV1 temporal unit that `gamut-avif` wraps in an
//! AVIF still image. The wider AV1 surface (lossy DCT/ADST, more intra modes, in-loop filters,
//! inter coding for image sequences) is tracked in `gamut-avif/STATUS.md`.
//!
//! Modules mirror the spec: [`headers`] = OBU framing + sequence/frame headers (AV1 §5.3/§5.5/§5.9),
//! `tile` = partition/prediction/coefficient coding (§5.11), `cdf` = default CDF + scan + context
//! tables (§9.2/§9.4/§8.3.2), [`quant`] = quantizer tables + dequant (§7.12).
#![forbid(unsafe_code)]

mod cdf;
mod encoder;
mod filter;
mod headers;
pub mod quant;
mod tile;
pub mod transform;

pub use encoder::{EncodedStill, ReconImage, encode_still_intra, encode_still_lossless_identity};
pub use headers::Av1StillConfig;
