# gamut-dng

`gamut-dng` is a pure-Rust DNG (Adobe Digital Negative) raw-image **encoder and decoder**.

## Goals

Part of the [gamut](../../README.md) workspace, this crate provides DNG writing (and a matching
raw decoder) that is:

- **Spec-faithful.** Implemented directly from the **DNG 1.7.1.0** specification
  ([`../../references/dng/DNG_Spec_1_7_1_0.pdf`](../../references/dng)) and conformance-checked
  against the **Adobe DNG SDK 1.7.1** as the authoritative oracle.
- **Memory-safe on hostile input.** `#![forbid(unsafe_code)]` — DNG's TIFF-derived, offset-driven
  structure is a classic source of parser exploits, so the decoder is built to be robust against
  malformed IFDs, offset loops, and truncation.
- **Built on shared primitives.** DNG is a profile of TIFF/EP, so its IFD container is the shared
  [`gamut-ifd`](../gamut-ifd) crate (the same spine [`gamut-tiff`](../gamut-tiff) uses); this crate
  adds only the DNG-specific tags, raw photometry, colour calibration, compression, and metadata.
- **Permissively licensed**, matching the royalty-free DNG format.

DNG is **natively a still-image** raw format — a good long-term fit for gamut's image-first focus.

## Scope

**Encoder-first** with a matching raw decoder (sample unpacking, decompression, and tag parsing).
Full demosaicing and colour rendering are a raw *processor's* job and stay out of scope — the
decoder returns the sensor samples (CFA mosaic or linear RGB) plus the parsed metadata.

## Usage

```rust,ignore
use gamut_dng::{CameraProfile, DngEncoder, RawImage};

// `raw` is a RawImage (CFA mosaic or LinearRaw); `profile` is a CameraProfile.
let mut dng = Vec::new();
DngEncoder::new()
    .encode(&raw, &profile, &mut dng)
    .expect("encode");
```

(The exact API lands incrementally — see Status.)

## Status

Implemented and conformance-checked against the Adobe DNG SDK (issue #109); see
[STATUS.md](STATUS.md) for the per-feature phase table.

- **Encode + decode**, both directions Adobe-validated: CFA mosaic and `LinearRaw` photometry;
  **uncompressed, Deflate/ZIP (8), and lossless JPEG (7)** compression; the colour-calibration
  profile (ColorMatrix1/2, CameraCalibration, ForwardMatrix, dual illuminant, AnalogBalance,
  BaselineExposure, profile identity); black/white levels, active area, default crop, and
  8/10/12/14/16-bit packing; an embedded RGB preview; EXIF/XMP/IPTC/ICC metadata; classic TIFF and
  **BigTIFF**.
- **Deferred** (tracked in `STATUS.md`): tiled layout, MD5 raw digests, the opcode lists, JPEG XL
  (depends on `gamut-jxl`), lossy JPEG, transparency/depth/semantic masks, and floating-point
  samples.

Correctness is pinned with the **Adobe DNG SDK** oracle — gamut-encode → `dng_validate` accepts the
file, and the SDK's stage-1 decode matches gamut's own decode pixel-for-pixel — plus the **libtiff**
oracle for the TIFF-container/preview layer and internal encode→decode round-trips on every path.

## License

Licensed under either of MIT or Apache-2.0 at your option.
