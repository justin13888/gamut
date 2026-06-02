//! ISO Base Media File Format (ISOBMFF) utilities for the AVIF container.
//!
//! M0 writes the minimal box set for a single still image item (AVIF v1.2.0 §9.1.1): `ftyp` then
//! a `meta` box holding `hdlr`/`pitm`/`iloc`/`iinf`(`infe`)/`iprp`(`ipco`+`ipma`), followed by an
//! `mdat` carrying the AV1 temporal unit. See [`write_avif_still`].
//!
//! Box byte layouts follow ISO/IEC 14496-12 (ISOBMFF) and ISO/IEC 23008-12 (HEIF) — which are
//! paywalled and not in `references/` — cross-checked against the public AVIF v1.2.0 box table and
//! libavif/ffmpeg output. The richer item set (alpha `auxl`, `grid`, transforms, sequence tracks)
//! is deferred per `gamut-avif/STATUS.md`.
#![forbid(unsafe_code)]

mod boxes;
mod writer;

pub use writer::{Av1cConfig, AvifStillImage, NclxColr, write_avif_still};
