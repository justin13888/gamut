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

No public API yet — implementation pending (issue #107). It will follow the same shape as the
other format crates: a `TiffEncoder` implementing [`gamut_core::Encoder`] and a `TiffDecoder`
implementing [`gamut_core::Decoder`], reachable through the umbrella crate's `tiff` feature.

## Status

Scaffolding — **under active implementation** (issue #107). See [STATUS.md](STATUS.md) for the
phase-by-phase progress of the TIFF 6.0 campaign.

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
