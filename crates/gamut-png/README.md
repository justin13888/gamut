# gamut-png

`gamut-png` is a pure-Rust, research-grade, space-efficient **PNG encoder**.

## Goals

Part of the [gamut](../../README.md) workspace, this crate writes PNG (Portable Network Graphics,
W3C 3rd edition) images that are:

- **Encoder-only.** PNG decoding is a solved problem, so — per gamut's encoder-first philosophy and
  issue #24 — this crate does not decode. Correctness is proven differentially against a vendored
  libpng (see Validation).
- **Space-efficient.** Built on [`gamut-deflate`](../gamut-deflate)'s zopfli-class compression, with
  adaptive scanline filtering and lossless bit-depth/palette reduction, targeting output sizes on par
  with the best PNG encoders. Encode time is a secondary concern at higher levels.
- **Memory-safe.** `#![forbid(unsafe_code)]`.

## Usage

```rust
use gamut_core::{Dimensions, EncodeImage, ImageRef, Rgb8};
use gamut_png::PngEncoder;

let image = ImageRef::<Rgb8>::new(&rgb, Dimensions::new(w, h)?)?;
let mut png = Vec::new();
PngEncoder::new().encode_image(image, &mut png)?;
```

## Status

Built incrementally; each phase is conformance-checked against libpng (see [STATUS.md](STATUS.md)).
Scope: all five colour types, bit depths 1/2/4/8/16, palette, the five scanline filters, lossless
reductions, the standard colour/text ancillary chunks, and embedded metadata (eXIf/iCCP/iTXt).
Out of scope: decoding, Adam7 interlacing, and animation (APNG).

## Validation

A differential oracle (`tooling/libpng-oracle`, a vendored static libpng) decodes the encoder's
output; the recovered pixels must match the source exactly, and output size is benchmarked against
libpng at maximum compression.

## License

Licensed under either of MIT or Apache-2.0 at your option.
