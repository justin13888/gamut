# gamut-webp

`gamut-webp` is a pure-Rust WebP **encoder and decoder** — a VP8/VP8L still-image bitstream wrapped
in a RIFF container.

## Goals

Part of the [gamut](../../README.md) workspace, this crate provides WebP encoding (and, unusually for
the encoder-first workspace, decoding) that is:

- **Memory-safe on hostile input.** `#![forbid(unsafe_code)]`, deleting the memory-corruption bug
  class behind libwebp's CVE record (e.g. the zero-click, wormable CVE-2023-4863). Because every
  WebP decoder in the Rust ecosystem ultimately wraps libwebp, a clean-slate safe decoder is worth
  carrying here.
- **Clean-slate from the spec.** Implemented directly from the VP8 / VP8L bitstream specs (see
  [`../../references/`](../../references)) rather than wrapping libwebp.
- **Layered on shared crates.** The container comes from [`gamut-riff`](../gamut-riff); the bit-level
  primitives from [`gamut-bitstream`](../gamut-bitstream); color handling from
  [`gamut-color`](../gamut-color).
- **Buildable anywhere `cargo` is.** No C, no nasm — cross-compiles cleanly (wasm32, aarch64, musl).
  (The differential test suite is the one exception: it builds libwebp via `libwebp-sys` as a
  dev-dependency, so running `cargo test` needs a C toolchain. The shipped library does not.)

WebP is one of gamut's three initial focus formats (alongside AVIF and JPEG).

## Usage

The public API follows the same shape as [`gamut-avif`](../gamut-avif): a `WebpEncoder` implementing
[`gamut_core::Encoder`] and a `WebpDecoder` implementing [`gamut_core::Decoder`], both reachable
through the umbrella crate's `webp` feature. **VP8L lossless** encode and decode are fully
implemented — `WebpEncoder::lossless` emits a conformant bit-exact stream, and `WebpDecoder` decodes
any conformant VP8L stream. The lossy VP8 path returns [`gamut_core::Error::Unsupported`] for now.

## Status

The intra-frame still-image surface and its milestones (M0 VP8L lossless → M1 VP8L full → M2 VP8
lossy → extended container) are tracked component-by-component in [`STATUS.md`](STATUS.md). Every
implemented component is validated against libwebp as an oracle via `libwebp-sys` (bit-exact for
lossless), backed by internal forward/inverse round-trips and the in-crate decoder.

**Non-core feature paths** are decided in [`STATUS.md`](STATUS.md#scope-decisions--non-core-feature-paths):
alpha/transparency (`VP8X` + `ALPH`) and color/metadata chunks (`ICCP` ICC profiles, `EXIF`, `XMP `)
are **in scope** — embedded on encode and preserved on decode. Animation (`ANIM`/`ANMF`) is **out of
scope** under the image-first charter (each frame is an independent keyframe, but multi-frame
sequences don't fit the single-image API); its chunks are tracked only for container completeness.

## License

Licensed under either of MIT or Apache-2.0 at your option.
