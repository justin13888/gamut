# gamut-color

`gamut-color` holds the color primitives the gamut codecs share: pixel formats, bit depths, chroma
subsampling, the CICP code points carried in nclx / AV1 sequence headers, and planar pixel buffers.

## Goals

Part of the [gamut](../../README.md) workspace, this crate exists to:

- **Centralize color management.** One place defines [`BitDepth`], [`ChromaSubsampling`], the CICP
  enums ([`ColourPrimaries`], [`TransferCharacteristics`], [`MatrixCoefficients`], [`ColorRange`]),
  and the [`Planar8`] buffer — so a color bug is fixed once, not re-implemented per format. (The
  interleaved pixel layouts are the typed `Pixel` markers in `gamut-core`.)
- **Model the full spec surface up front.** The M0 AVIF path uses only a narrow slice (8-bit RGB in,
  mapped to identity `mc = 0` 4:4:4 planes), but the enums intentionally cover the wider range of
  formats, bit depths, subsamplings, and CICP code points so later milestones (M2 pixel formats, M4
  HDR — see [`gamut-avif/STATUS.md`](../gamut-avif/STATUS.md)) extend without reshaping the types.
- **Match the spec code points exactly.** CICP values mirror the H.273 / AV1 sequence-header code
  points so they round-trip through `av1C`/`colr` and AV1 headers unchanged.
- **Stay memory-safe.** `#![forbid(unsafe_code)]`.

## Usage

```rust
use gamut_color::Planar8;

// 8-bit interleaved RGB -> identity (mc = 0) 4:4:4 planes (Y=G, U=B, V=R).
let width = 2;
let height = 2;
let rgb: Vec<u8> = vec![0; width * height * 3];
let planes = Planar8::from_rgb8_identity(&rgb, width as u32, height as u32).expect("valid input");
assert_eq!(planes.width(), 2);
let _y = planes.plane(0); // luma plane
```

## Status

M0 exercises the 8-bit RGB ↔ identity 4:4:4 conversion ([`Planar8::from_rgb8_identity`] /
[`Planar8::to_rgb8_identity`]) plus the CICP tables the AVIF `colr` box needs. The remaining
formats, bit depths, and subsamplings are modeled in the type system but not yet wired into an
encode path; they land with the milestones tracked in
[`gamut-avif/STATUS.md`](../gamut-avif/STATUS.md).

## License

Licensed under either of MIT or Apache-2.0 at your option.
