# gamut-core

`gamut-core` defines the shared vocabulary every gamut codec is built on: the
[`Encoder`]/[`Decoder`] traits, the [`Dimensions`] type, and the common [`Error`] surface.

## Goals

Part of the [gamut](../../README.md) workspace, this crate exists to provide a single, format-neutral
foundation so that:

- **Every codec speaks one API.** A caller drives `gamut-avif`, `gamut-webp`, or any future format
  through the same [`Encoder`]/[`Decoder`] traits and gets back the same [`Error`] type, regardless
  of format.
- **The dependency graph stays acyclic.** This crate has **no** dependency on the format crates;
  everything else in the workspace depends on it, never the other way around.
- **Errors are typed, never panicking.** [`Error`] is a `thiserror` enum marked `#[non_exhaustive]`,
  so new variants land without a breaking change and library paths return errors instead of
  `unwrap()`/`expect()`.
- **Hot paths stay allocation-conscious.** [`Encoder::encode`] appends to a caller-provided
  `&mut Vec<u8>` rather than allocating a fresh buffer, so callers can reuse scratch space.
- **Memory-safe end to end.** `#![forbid(unsafe_code)]`.

## Usage

```rust
use gamut_core::{Dimensions, Encoder, Error, Result};

struct MyEncoder;

impl Encoder for MyEncoder {
    fn encode(&self, pixels: &[u8], dims: Dimensions, out: &mut Vec<u8>) -> Result<usize> {
        if pixels.len() != (dims.width * dims.height * 3) as usize {
            return Err(Error::InvalidInput("pixel buffer does not match dimensions"));
        }
        let start = out.len();
        out.extend_from_slice(pixels); // a real codec would compress here
        Ok(out.len() - start)
    }
}
```

## Status

Stable foundation. The trait surface ([`Encoder`], [`Decoder`]), [`Dimensions`], [`Result`], and the
[`Error`] variants (`InvalidInput`, `Unsupported`) are in place and used across the implemented M0
path. New `Error` variants and image-buffer helpers are added here as later milestones need them; the
`#[non_exhaustive]` enum keeps that additive.

## License

Licensed under either of MIT or Apache-2.0 at your option.
