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
//! with a matching raw decoder (sample unpacking + decompression + tag parsing); full demosaicing
//! and colour rendering are a raw *processor's* job and stay out of scope. See `STATUS.md` for the
//! per-feature implementation status and the deferred tail (JPEG XL, lossy JPEG, the standard
//! opcode library, masks/depth maps).
//!
//! Memory-safe on hostile input: `#![forbid(unsafe_code)]` — like TIFF, DNG's offset-driven
//! structure is a classic parser-exploit surface, so the decoder is built to resist malformed
//! IFDs, offset loops, and truncation.
#![forbid(unsafe_code)]

pub mod encoder;
pub mod profile;
pub mod raw;
pub mod tags;
pub mod values;

mod bitpack;
mod preview;
mod writer;

// The shared error/result/dimension types every gamut codec speaks, re-exported so callers need
// not also depend on `gamut-core` directly, along with the byte-order selector from the IFD core.
// The decoder and the compression schemes land in subsequent phases — see `STATUS.md`.
pub use gamut_core::{Dimensions, Error, Result};
pub use gamut_ifd::ByteOrder;

pub use encoder::DngEncoder;
pub use profile::CameraProfile;
pub use raw::RawImage;
pub use values::{
    CalibrationIlluminant, CfaLayout, Compression, PhotometricInterpretation, Predictor,
    PreviewColorSpace, ProfileEmbedPolicy, SampleFormat,
};
