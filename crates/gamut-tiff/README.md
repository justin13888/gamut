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

**Under active implementation** (issue #107). Baseline TIFF (uncompressed/PackBits; grayscale, RGB,
palette, bilevel) is done and conformance-checked against libtiff; the extensions are landing
incrementally. See [STATUS.md](STATUS.md) for phase-by-phase progress.

## Roadmap

Full TIFF 6.0 (§1–23), delivered as a stack of small PRs:

- **Baseline** — TIFF structure / IFD model, uncompressed grayscale & RGB (the keystone), bilevel
  & PackBits, palette colour, baseline field reference + CLI.
- **Compression extensions** — Modified Huffman, LZW + differencing predictor, CCITT T.4 / T.6
  fax.
- **Layout & colour extensions** — tiled images, planar config, associated alpha, 16-bit/float
  samples, CMYK, YCbCr, RGB colorimetry, CIE L\*a\*b\*, multi-page documents.
- **JPEG-in-TIFF** (§22) and finalization (robustness corpus, interop sweep) last.

Correctness is pinned with a differential oracle against **libtiff**: gamut-encode → libtiff-decode
and libtiff-encode → gamut-decode must agree pixel-for-pixel on every lossless path.

## License

Licensed under either of MIT or Apache-2.0 at your option.
