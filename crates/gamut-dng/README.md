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

In active development against issue #109; see [STATUS.md](STATUS.md) for the per-feature
phase table and the deferred tail.

- **In scope:** uncompressed, Lossless JPEG (7), and Deflate/ZIP (8) compression; CFA and
  `LinearRaw` photometry; the full colour/calibration tag set; black/white levels and bit-depth
  packing; embedded preview; EXIF/XMP/IPTC/ICC metadata; classic TIFF and BigTIFF; strips and
  tiles; MD5 raw digests; the opcode-list container.
- **Deferred** (tracked in `STATUS.md`): JPEG XL compression (depends on `gamut-jxl`), lossy JPEG,
  the standard opcode library, transparency/depth/semantic masks, and floating-point samples.

Correctness is pinned with the **Adobe DNG SDK** oracle (gamut-encode → `dng_validate` must accept
the file; Adobe sample DNGs → gamut-decode must match), the **libtiff** oracle for the
TIFF-container/strip layer, and internal encode→decode round-trips on every lossless path.

## License

Licensed under either of MIT or Apache-2.0 at your option.
