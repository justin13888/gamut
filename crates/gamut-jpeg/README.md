# gamut-jpeg

`gamut-jpeg` is a pure-Rust, spec-compliant **JPEG-1** (ISO/IEC 10918-1 | ITU-T T.81) still-image
codec.

## Goals

Part of the [gamut](../../README.md) workspace, this crate reads and writes JPEG-1 images that are:

- **Spec-compliant.** Implemented directly against ITU-T T.81 (the core codec) and T.871 (JFIF),
  with clause citations in the source. Correctness is proven differentially against libjpeg-turbo
  (see Validation).
- **Encoder + decoder.** Unlike the workspace's encoder-only PNG crate, JPEG is a two-way format:
  a baseline/extended sequential DCT Huffman **encoder and decoder** ship together, with the
  progressive decoder and encoder landing in later phases (see [STATUS.md](STATUS.md)).
- **Memory-safe.** `#![forbid(unsafe_code)]`.

## Usage

Encode an RGB image to a JFIF stream:

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

Decode a JPEG back to pixels (grayscale is replicated across channels; YCbCr/RGB present as RGB;
four-component CMYK/YCCK decode via `Cmyk8`):

```rust
use gamut_core::{DecodeImage, ImageBuf, Rgb8};
use gamut_jpeg::JpegDecoder;

# fn demo(jpeg: &[u8]) -> Result<(), gamut_core::Error> {
let image: ImageBuf<Rgb8> = JpegDecoder::new().decode_image(jpeg)?;
let info = gamut_jpeg::info(jpeg)?; // dimensions / components / process, without decoding
# let _ = (image, info);
# Ok(())
# }
```

## Status

Built incrementally; each phase is conformance-checked against libjpeg-turbo (see
[STATUS.md](STATUS.md)). Shipping now: the **baseline (SOF0) / extended-sequential (SOF1) 8-bit DCT
Huffman encoder and decoder**. The encoder writes grayscale and JFIF YCbCr with 4:4:4 / 4:2:2 /
4:2:0 chroma subsampling, standard (Annex K) tables, and optional restart intervals. The decoder
reads any spec-valid sequential stream — grayscale, YCbCr, RGB, and CMYK/YCCK (via the JFIF APP0 /
Adobe APP14 hints), interleaved or non-interleaved scans, restart intervals, and DNL-defined
heights — and never panics on malformed input. The progressive decoder and encoder are scoped for
later phases.

Out of scope (documented in [STATUS.md](STATUS.md)): 12-bit precision, arithmetic coding
(SOF9/10), lossless (SOF3), hierarchical (SOF5–7), SPIFF/T.84 extensions, and T.872 printing
conventions.

## Validation

A live differential gate (`tests/oracle.rs`) cross-checks against a vendored, dev-only
**libjpeg-turbo 3.2.0** static build (`tooling/libjpeg-oracle`) in both directions:

- **Encode** — gamut encodes, libjpeg-turbo decodes; the recovered pixels match the source within
  the format's lossy tolerance (measured: gray/4:4:4 within a few codes, subsampled above a PSNR
  floor). This proves gamut emits spec-valid streams the canonical reference decoder reads back.
- **Decode** — libjpeg-turbo encodes (including non-standard optimized Huffman tables and restart
  markers), gamut decodes and matches libjpeg-turbo's own decode of the same stream, isolating
  entropy/dequant/IDCT correctness from lossy encode error.

The byte stream is additionally cross-checked against the vendored T.873 reference software for
spec-exact behaviour. Running the gate builds the C oracle, so the tests need the
`third_party/libjpeg-turbo` submodule checked out and a C toolchain on `PATH`.

## License

Licensed under either of MIT or Apache-2.0 at your option.
