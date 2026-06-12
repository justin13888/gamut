# gamut-core

`gamut-core` defines the shared vocabulary every gamut codec is built on: the
[`EncodeImage`]/[`DecodeImage`] traits, the branded [`ImageRef`]/[`ImageBuf`] pixel buffers, the
[`Pixel`] marker types, the [`Dimensions`] type, and the common [`Error`] surface.

## Goals

Part of the [gamut](../../README.md) workspace, this crate exists to provide a single, format-neutral
foundation so that:

- **Every codec speaks one API.** A caller drives `gamut-avif`, `gamut-webp`, or any future format
  through the same [`EncodeImage`]/[`DecodeImage`] traits and gets back the same [`Error`] type,
  regardless of format. A codec implements them only for the pixel layouts it supports, so handing
  it an unsupported format is a compile error, not a runtime check.
- **The dependency graph stays acyclic.** This crate has **no** dependency on the format crates;
  everything else in the workspace depends on it, never the other way around.
- **Errors are typed, never panicking.** [`Error`] is a `thiserror` enum marked `#[non_exhaustive]`,
  so new variants land without a breaking change and library paths return errors instead of
  `unwrap()`/`expect()`.
- **Hot paths stay allocation-conscious.** [`EncodeImage::encode_image`] appends to a caller-provided
  `&mut Vec<u8>` rather than allocating a fresh buffer, so callers can reuse scratch space.
- **Memory-safe end to end.** `#![forbid(unsafe_code)]`.

## Usage

```rust
use gamut_core::{Dimensions, EncodeImage, ImageRef, Rgb8, Result};

struct MyEncoder;

impl EncodeImage<Rgb8> for MyEncoder {
    fn encode_image(&self, image: ImageRef<'_, Rgb8>, out: &mut Vec<u8>) -> Result<usize> {
        // `image` is already validated: its sample count matches its dimensions.
        let start = out.len();
        out.extend_from_slice(image.as_samples()); // a real codec would compress here
        Ok(out.len() - start)
    }
}

// Build a validated, branded RGB image and hand it to the encoder.
let dims = Dimensions::new(2, 2).unwrap();
let rgb = vec![0u8; dims.sample_count(3).unwrap()];
let mut out = Vec::new();
MyEncoder
    .encode_image(ImageRef::<Rgb8>::new(&rgb, dims).unwrap(), &mut out)
    .unwrap();
```

## Status

Stable foundation. The trait surface ([`EncodeImage`], [`DecodeImage`]), the branded [`ImageRef`] /
[`ImageBuf`] buffers and [`Pixel`] markers, [`Dimensions`], [`Result`], and the
[`Error`] variants (`InvalidInput`, `Unsupported`) are in place and used across the implemented M0
path. New `Error` variants and image-buffer helpers are added here as later milestones need them; the
`#[non_exhaustive]` enum keeps that additive.

## License

Licensed under either of MIT or Apache-2.0 at your option.
