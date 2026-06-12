# gamut-tiff

`gamut-tiff` is a pure-Rust TIFF 6.0 (Tagged Image File Format) image **encoder and decoder**.

## Goals

Part of the [gamut](../../README.md) workspace, this crate provides TIFF reading and writing that
is:

- **Memory-safe on hostile input.** `#![forbid(unsafe_code)]` — TIFF's offset-driven structure is
  a classic source of parser exploits, so the decoder is built to be robust against malformed
  IFDs, offset loops, and truncation.
- **Clean-slate from the spec.** Implemented directly from the TIFF 6.0 specification
  ([`../../references/tiff/tiff6.pdf`](../../references/tiff)) rather than wrapping libtiff.
- **Self-contained.** TIFF's Image File Directory (IFD) / tag structure *is* its container, so —
  unlike [`gamut-avif`](../gamut-avif)/[`gamut-heic`](../gamut-heic) (ISOBMFF) or
  [`gamut-webp`](../gamut-webp) (RIFF) — this crate needs no separate container crate. It builds
  only on [`gamut-color`](../gamut-color), [`gamut-dsp`](../gamut-dsp), and
  [`gamut-bitstream`](../gamut-bitstream).
- **Permissively licensed**, matching the royalty-free TIFF format.

Unlike the video-derived still-image codecs in the workspace, TIFF is **natively a still-image
format** — a good long-term fit for gamut's image-first focus.

## Usage

[`TiffEncoder`] (implementing [`gamut_core::Encoder`]) writes 8-bit grayscale, RGB, palette, and
1-bit bilevel images — uncompressed or PackBits — and [`TiffDecoder`] (implementing
[`gamut_core::Decoder`]) reads them back, both reachable through the umbrella crate's `tiff`
feature:

```rust
use gamut_core::Dimensions;
use gamut_tiff::{Compression, TiffEncoder};

let mut tiff = Vec::new();
TiffEncoder::new()
    .with_compression(Compression::PackBits)
    .encode_rgb8(&rgb, Dimensions { width, height }, &mut tiff)
    .expect("encode");
```

More colour modes and compression schemes are landing incrementally (see Status).

## Status

**Implemented and conformance-checked against libtiff** (issue #107):

- **Structure** — byte-order header, IFD/tag read & write, strips and tiles, multi-page documents.
- **Colour modes** (8-bit) — grayscale, RGB, RGBA (alpha), palette, CMYK, and 1-bit bilevel.
- **Compression** — uncompressed, PackBits, LZW (+ horizontal differencing predictor), and the
  bilevel CCITT schemes Modified Huffman (Group 3 1-D) and Group 4 (T.6).
- The decoder is hardened against hostile input (`#![forbid(unsafe_code)]`, a size cap, and a
  byte-flip fuzz corpus).

**Not yet implemented** (see [STATUS.md](STATUS.md)): YCbCr (§21), CIE L\*a\*b\* / RGB colorimetry
(§20, §23), JPEG-in-TIFF (§22), and smaller items (CCITT Group 3 2-D, planar config, 16-bit/float
samples, halftone hints).

## Roadmap

The remaining TIFF 6.0 features each land as a follow-up PR that plugs into the same strip/tile
pipeline and libtiff oracle: the colour spaces (YCbCr, L\*a\*b\*) need `gamut-color` conversions
matched to libtiff's integer math; JPEG-in-TIFF needs a baseline DCT codec and a `libjpeg`-enabled
oracle build.

Correctness is pinned with a differential oracle against **libtiff**: gamut-encode → libtiff-decode
and libtiff-encode → gamut-decode must agree pixel-for-pixel on every lossless path.

## License

Licensed under either of MIT or Apache-2.0 at your option.
