# gamut-jpeg

`gamut-jpeg` is a pure-Rust, spec-compliant **JPEG-1** (ISO/IEC 10918-1 | ITU-T T.81) still-image
codec.

## Goals

Part of the [gamut](../../README.md) workspace, this crate reads and writes JPEG-1 images that are:

- **Spec-compliant.** Implemented directly against ITU-T T.81 (the core codec) and T.871 (JFIF),
  with clause citations in the source. Correctness is proven differentially against libjpeg-turbo
  (see Validation).
- **Encoder + decoder.** Unlike the workspace's encoder-only PNG crate, JPEG is a two-way format:
  a baseline sequential DCT Huffman **encoder** ships first (this phase), with the sequential and
  progressive **decoders** landing in later phases (see [STATUS.md](STATUS.md)).
- **Memory-safe.** `#![forbid(unsafe_code)]`.

## Usage

```rust
use gamut_core::{Dimensions, EncodeImage, ImageRef, Rgb8};
use gamut_jpeg::{ChromaSubsampling, JpegEncoder};

let (w, h) = (16, 16);
let rgb = vec![0u8; (w * h * 3) as usize];
let image = ImageRef::<Rgb8>::new(&rgb, Dimensions::new(w, h)?)?;
let mut jpeg = Vec::new();
JpegEncoder::new()
    .with_quality(85)
    .with_subsampling(ChromaSubsampling::Ycbcr420)
    .encode_image(image, &mut jpeg)?;
# Ok::<(), gamut_core::Error>(())
```

## Status

Built incrementally; each phase is conformance-checked against libjpeg-turbo (see
[STATUS.md](STATUS.md)). This first phase ships the **baseline sequential DCT Huffman encoder**
(SOF0, 8-bit): grayscale and JFIF YCbCr with 4:4:4 / 4:2:2 / 4:2:0 chroma subsampling, standard
(Annex K) quantization and Huffman tables, and optional restart intervals, emitting JFIF
interchange-format streams. The sequential and progressive decoders, a progressive encoder, and
CMYK/YCCK + Adobe APP14 handling are scoped for later phases.

Out of scope (documented in [STATUS.md](STATUS.md)): 12-bit precision, arithmetic coding
(SOF9/10), lossless (SOF3), hierarchical (SOF5–7), DNL, SPIFF/T.84 extensions, and T.872 printing
conventions.

## Validation

A differential oracle (a vendored, dev-only libjpeg-turbo, landing with the decoder phase) decodes
the encoder's output; the recovered pixels must match within the format's lossy tolerance, and the
byte stream is cross-checked against the vendored T.873 reference software for spec-exact behaviour.

## License

Licensed under either of MIT or Apache-2.0 at your option.
