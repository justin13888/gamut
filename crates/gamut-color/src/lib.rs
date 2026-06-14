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
//!
//! # API layout
//!
//! Every module is public, so the full surface — including the colour-science long tail (the `M1` /
//! `M2` matrices, the `*_standard` transfer-curve variants, the matrix derivations) — is reachable
//! and grouped under its module. For convenience the crate root additionally re-exports the items
//! most consumers name directly: the CICP enums, [`BitDepth`] / [`ChromaSubsampling`],
//! [`Planar8`] / [`Yuv420`], the [`clip_pixel`] / [`rgb_to_ycbcr`] helpers, and the colour-science
//! entry types [`Gamut`] and [`SourceProfile`].
//!
//! # Implemented vs. modeled
//!
//! Many enum variants model the full spec surface but are **not yet wired into an encode path** —
//! they exist so later milestones extend without reshaping the types. As of this release:
//!
//! - **Implemented:** 8-bit ([`BitDepth::Eight`]) RGB → identity 4:4:4 ([`ChromaSubsampling::Cs444`],
//!   [`MatrixCoefficients::Identity`]) planes; the CICP code-point tables; BT.601 YCbCr 4:2:0
//!   ([`ycbcr`]); and the `f64` colour science ([`transfer`], [`oklab`], [`matrix`], [`gamut_map`],
//!   [`profile`]) for the sRGB, Display P3, Adobe RGB, BT.2020 and ProPhoto gamuts.
//! - **Modeled but deferred:** 10/12-bit ([`BitDepth::Ten`] / [`BitDepth::Twelve`]); the subsampled
//!   formats ([`ChromaSubsampling::Cs422`] / `Cs420` / `Cs400`); the non-identity matrix coefficients
//!   ([`MatrixCoefficients::Bt709`] / `Bt601` / `Bt2020Ncl` / `YCgCo`); and the HLG / BT.709 transfer
//!   curves ([`eotf_for`](transfer::eotf_for) returns `None` for these). These land with the
//!   milestones tracked in `gamut-avif/STATUS.md`.
#![forbid(unsafe_code)]

pub mod cicp;
pub mod format;
pub mod gamut_map;
mod linalg;
pub mod matrix;
pub mod oklab;
pub mod pixel;
pub mod planar;
pub mod profile;
pub mod transfer;
pub mod ycbcr;

pub use cicp::{ColorRange, ColourPrimaries, MatrixCoefficients, TransferCharacteristics};
pub use format::{BitDepth, ChromaSubsampling};
pub use oklab::Gamut;
pub use pixel::{clip_pixel, clip_pixel8};
pub use planar::Planar8;
pub use profile::SourceProfile;
pub use ycbcr::{Yuv420, rgb_to_ycbcr, ycbcr_to_rgb};
