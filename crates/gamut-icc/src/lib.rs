//! `gamut-icc` — ICC color profile parsing and serialization.
//!
//! An ICC profile is the self-describing colour-characterization blob embedded in images (the WebP
//! `ICCP` chunk, the AVIF/HEIF `colr` box of type `prof`, a JPEG `APP2` segment): a 128-byte header,
//! a tag table, then the tag element data the table points at. It is a flat, offset-indexed binary
//! format that needs neither the TIFF/IFD machinery nor XML, so this crate depends only on
//! [`gamut_core`] (plus `md-5`, for the §7.2.18 profile ID).
//!
//! Layouts follow **ICC.1:2022** (profile version 4.4, equivalent to ISO 15076-1; see
//! `references/icc`). Profile **v2** — still the most common version in real images — is supported,
//! including its legacy `textDescriptionType`.
//!
//! # Reading and writing
//!
//! [`IccProfile::parse`] decodes a profile and [`IccProfile::to_bytes`] re-serializes it; look tags
//! up with [`IccProfile::get`], optionally via the [`KnownTag`] catalogue. [`IccReader`] and
//! [`IccWriter`] carry options (strict parsing; profile-ID recomputation).
//! [`IccProfile::validate`] reports any ICC.1:2022 §8 required tags missing for the profile's
//! device class.
//!
//! ```no_run
//! use gamut_icc::{IccProfile, KnownTag, TagData};
//!
//! # fn demo(bytes: &[u8]) -> Result<(), gamut_core::Error> {
//! let profile = IccProfile::parse(bytes)?;
//! if let Some(TagData::Xyz(white)) = profile.get(KnownTag::MediaWhitePoint) {
//!     println!("media white point: {:?}", white[0].to_f64());
//! }
//! let serialized = profile.to_bytes(); // spec-valid bytes, ready to re-embed
//! # let _ = serialized;
//! # Ok(())
//! # }
//! ```
//!
//! # Scope
//!
//! Every ICC.1:2022 §10 element type is decoded semantically (see [`TagData`]). Any element type not
//! defined in §10 — iccMAX's `multiProcessElementsType`, or private/unregistered types — is
//! preserved verbatim as [`TagData::Raw`], so every profile round-trips losslessly regardless of
//! what it carries. Applying a profile's transform (a CMM), and building transforms from
//! [`gamut_color`](https://docs.rs/gamut-color), are out of scope — the `to_f64`/`eval` accessors
//! are the seam for that — as is **iccMAX** (`ICC.2`), a separate next-generation profile format.
#![forbid(unsafe_code)]

mod bytes;
mod cicp;
mod colorant;
mod curve;
mod data;
mod dict;
mod header;
mod lut;
mod measurement;
mod mluc;
mod named_color;
mod primitives;
mod profile;
mod reader;
mod sequence;
mod tag_types;
mod tags;
mod validate;
mod writer;

pub use cicp::Cicp;
pub use colorant::{Colorant, ColorantOrder, ColorantTable};
pub use curve::{Curve, CurveOrParametric, ParametricCurve};
pub use data::DataElement;
pub use dict::{Dict, DictEntry};
pub use header::{
    ColorSpace, DeviceClass, ProfileHeader, ProfileId, ProfileVersion, RenderingIntent,
};
pub use lut::{Clut, ClutPrecision, Lut8, Lut16, LutAToB, LutBToA, Matrix3x3, Matrix3x4};
pub use measurement::{Chromaticity, Measurement, ViewingConditions};
pub use mluc::{Mluc, MlucRecord, TextDescription};
pub use named_color::{NamedColor, NamedColor2};
pub use primitives::{DateTime, S15Fixed16, Signature, U8Fixed8, U16Fixed16, XyzNumber};
pub use profile::IccProfile;
pub use reader::IccReader;
pub use sequence::{
    DescriptionText, ProfileDescription, ProfileIdentifier, ProfileSequenceDesc,
    ProfileSequenceIdentifier, Response16, ResponseCurve, ResponseCurveSet16,
};
pub use tag_types::TagData;
pub use tags::KnownTag;
pub use validate::Conformance;
pub use writer::IccWriter;
