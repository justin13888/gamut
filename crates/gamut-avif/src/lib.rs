//! AVIF (AV1 Image File Format) encoder — AV1 intra-frame bitstreams wrapped in an ISOBMFF/MIAF
//! container.
//!
//! The public surface is [`AvifEncoder`], which implements [`gamut_core::EncodeImage<Rgb8>`], so the
//! input is a typed [`ImageRef`](gamut_core::ImageRef) and handing it an unsupported pixel layout is
//! a compile error. The crate is orchestration only: [`gamut_color`] maps pixels to identity-matrix
//! 4:4:4 planes, [`gamut_av1`] encodes the AV1 temporal unit, and [`gamut_isobmff`] writes the
//! container.
//!
//! # Examples
//!
//! ```
//! use gamut_avif::AvifEncoder;
//! use gamut_core::{Dimensions, EncodeImage, ImageRef, Rgb8};
//!
//! // A 2×2 8-bit RGB image, row-major (red, green, blue, yellow).
//! let pixels = [255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0];
//! let image = ImageRef::<Rgb8>::new(&pixels, Dimensions { width: 2, height: 2 })?;
//!
//! // Lossless by default; `AvifEncoder::lossy(quality)` trades fidelity for a smaller file.
//! let avif = AvifEncoder::new().encode_to_vec(image)?;
//! assert_eq!(&avif[4..8], b"ftyp");
//! # Ok::<(), gamut_core::Error>(())
//! ```
//!
//! # Supported / deferred
//!
//! gamut is image-first, so only the still-image (intra) subset of AV1 is in scope — no sequences or
//! animation. **Supported:** 8-bit RGB input; **lossless** (the default, decoded output bit-exact to
//! the input) and **lossy** ([`AvifEncoder::lossy`], `quality` `0..=100`) AV1 intra coding at
//! identity-matrix 4:4:4; and `irot`/`imir` display orientation ([`AvifEncoder::with_rotation`] /
//! [`AvifEncoder::with_mirror`]). Output is validated end-to-end against `libavif` (its dav1d-backed
//! reference container decoder); the wrapped AV1 bitstream is cross-checked against `libaom` — the
//! AV1 reference codec — and `dav1d` via [`gamut_av1`].
//!
//! **Deferred, planned** (tracked row-by-row against the specs in `STATUS.md`, whose disposition
//! ledger is the authority): alpha / RGBA, 10/12-bit and 4:2:0/4:2:2, non-identity colour matrices
//! and ICC / Exif / XMP, HDR (PQ/HLG and the HDR metadata properties), `grid` / `tmap` (gain-map) /
//! `sato` derivations and the remaining container transforms, layered/progressive still images,
//! encoder speed / rate control, and an AVIF **decoder**. All of these land semver-minor — the v1
//! surface is designed so no deferred feature reshapes it.
//!
//! **Permanently out of scope** (workspace charter — gamut is image-first): image sequences and
//! tracks (the `avis`/`avio` brands) and AV1 inter-frame coding.
#![forbid(unsafe_code)]

mod av1c;
mod config;
mod encoder;
mod obu;
mod transform;

pub use av1c::{Av1Config, ChromaFormat};
pub use config::{AvifConfig, AvifMode};
pub use encoder::AvifEncoder;
pub use gamut_core::Dimensions;
pub use obu::{Obu, ObuHeader, ObuIter, ObuType, iter_obus};
pub use transform::{Mirror, Rotation};
