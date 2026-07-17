//! `gamut-dng` — a pure-Rust DNG (Adobe Digital Negative) raw-image **encoder and decoder**.
//!
//! DNG is Adobe's open raw format, built as a profile of **TIFF/EP**: an Image File Directory
//! (IFD) tree carries the camera's sensor samples plus the colour-calibration, geometry, and
//! metadata a raw processor needs to render them. Because the container *is* a TIFF, this crate
//! builds on the shared [`gamut_ifd`](https://crates.io/crates/gamut-ifd) IFD core (the same spine
//! `gamut-tiff` uses) and adds only the DNG-specific layers on top: the raw photometries
//! (`CFA` mosaic / `LinearRaw`), the colour/calibration model, the raw compression schemes, and
//! the embedded metadata.
//!
//! ## Structure
//!
//! A DNG's defining shape is an IFD **tree**, not a flat chain: IFD0 holds a small
//! preview/thumbnail and points, through the `SubIFDs` tag, at the full-resolution raw image in a
//! **sub-IFD**; EXIF lives in another sub-IFD. The encoder lays this tree out over `gamut-ifd`'s
//! tree-aware writer and composes the strip/tile pixel data around it; the decoder walks the tree
//! back to the raw samples and the parsed tags.
//!
//! ## Scope
//!
//! Reference: the **DNG 1.7.1.0 specification** (`references/dng/DNG_Spec_1_7_1_0.pdf`), validated
//! against the **Adobe DNG SDK 1.7.1** as the authoritative oracle. The crate is **encoder-first**
//! with a matching raw decoder (sample unpacking + decompression + tag parsing): decode returns
//! the *stored* sensor values, with the spec's chapter-5 raw-to-linear mapping available as the
//! explicit opt-in [`RawImage::to_linear`] (linearization table, black pattern + deltas, rescale
//! — differentially gated against the SDK's stage-2 image). The level model is the typed
//! [`RawLevels`]; opcode lists are typed [`OpcodeList`] containers; and the SOF3 codec is the
//! public [`lossless_jpeg`] module. Full demosaicing and colour rendering are a raw *processor's*
//! job and stay out of scope. See `STATUS.md` for the per-feature implementation status and the
//! deferred tail (tiles, JPEG XL, lossy JPEG, the standard opcode *processing* library,
//! masks/depth maps).
//!
//! Memory-safe on hostile input: `#![forbid(unsafe_code)]` — like TIFF, DNG's offset-driven
//! structure is a classic parser-exploit surface, so the decoder is built to resist malformed
//! IFDs, offset loops, and truncation.
#![forbid(unsafe_code)]

pub mod decoder;
pub mod deconstruct;
pub mod encoder;
pub mod gain_map;
pub mod levels;
pub mod linearize;
pub mod lossless_jpeg;
pub mod metadata;
pub mod opcode;
pub mod profile;
pub mod raw;
pub mod subimage;
pub mod tags;
pub mod values;

mod bitpack;
mod compression;
mod jxl;
mod preview;
mod writer;

// The shared error/result/dimension types every gamut codec speaks, re-exported so callers need
// not also depend on `gamut-core` directly, along with the byte-order selector from the IFD core.
pub use decoder::{DecodedDng, DngDecoder, RawTag};
pub use deconstruct::{Anomaly, DeconstructReport, Severity, UnknownTag, deconstruct};
pub use encoder::DngEncoder;
pub use gain_map::{GainValues, ProfileGainTableMap};
pub use gamut_core::{Dimensions, Error, Result};
// `Value` is part of the decode surface: `RawTag` carries unmodelled fields as this typed enum.
pub use gamut_ifd::{ByteOrder, Value};
pub use levels::RawLevels;
pub use linearize::LinearImage;
pub use lossless_jpeg::LosslessJpeg;
pub use metadata::{DngMetadata, ExifMetadata};
pub use opcode::{Opcode, OpcodeList};
pub use profile::CameraProfile;
pub use raw::RawImage;
pub use subimage::{
    DepthInfo, MaskSubArea, SemanticMaskInfo, SubImage, SubImageData, SubImageKind,
};
pub use values::{
    CalibrationIlluminant, CfaLayout, Compression, PhotometricInterpretation, Predictor,
    PreviewColorSpace, ProfileEmbedPolicy, SampleFormat,
};
