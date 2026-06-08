# gamut-avif

`gamut-avif` is a pure-Rust, memory-safe AVIF encoder that wraps AV1 intra-frame bitstreams in an
ISOBMFF/MIAF container.

This is the high-level crate most users want: give it pixels, get a complete `.avif` file. It is
orchestration only — [`gamut-av1`](../gamut-av1) does the AV1 coding and
[`gamut-isobmff`](../gamut-isobmff) writes the container.

## Goals

Part of the [gamut](../../README.md) workspace, this crate exists to provide AVIF **encoding** that
is:

- **Memory-safe on hostile input.** `#![forbid(unsafe_code)]` end to end — the entire encode and
  container path is safe Rust, deleting the spatial/temporal memory-corruption bug class that has
  repeatedly bitten the C image codecs.
- **Buildable anywhere `cargo` is.** No C, no autotools/CMake, no nasm — just Rust, so it
  cross-compiles cleanly (wasm32, aarch64, musl) for serverless/edge image optimization.
- **Encoder-first and size-first.** The product is the encoder and the bytes it emits; the
  space/time tradeoff of each mode is documented as it lands.
- **Clean-slate from the official specs.** Implemented directly from the AV1 Bitstream &
  Decoding Process Specification and the AVIF / AV1-ISOBMFF bindings (see `../../references/`), so
  it is auditable and forkable rather than a wrapper over libaom/libavif.
- **Permissively licensed** (MIT OR Apache-2.0), matching the royalty-free AV1/AVIF formats.

It builds on the workspace's shared primitives: [`gamut-color`](../gamut-color) (pixel formats /
CICP), [`gamut-dsp`](../gamut-dsp) (transforms), [`gamut-bitstream`](../gamut-bitstream) (bit
writer + AV1 symbol coder), [`gamut-av1`](../gamut-av1) (the AV1 keyframe encoder), and
[`gamut-isobmff`](../gamut-isobmff) (the container).

## Usage

```rust
use gamut_avif::AvifEncoder;
use gamut_core::{Dimensions, Encoder};

let width = 64;
let height = 64;
let rgb: Vec<u8> = vec![0; width * height * 3]; // 8-bit interleaved RGB

let mut avif = Vec::new();
AvifEncoder::new()
    .encode_rgb8(&rgb, Dimensions { width: width as u32, height: height as u32 }, &mut avif)
    .expect("encode");
std::fs::write("out.avif", &avif).unwrap();
```

`AvifEncoder` also implements the [`gamut_core::Encoder`] trait (assuming the same 8-bit
interleaved RGB layout).

## Status

Today (milestone **M0**) the encoder produces **lossless** still images: 8-bit RGB mapped to AV1
identity-matrix 4:4:4 (so the decoded image is bit-exact to the input), wrapped as a single `av01`
item. The space/time tradeoff is the obvious one — lossless output is exact but large; it makes no
attempt to be compact yet, and correctness is the priority. Output is verified against real
decoders (`avifdec`, `dav1d`).

Everything beyond M0 — lossy intra (DCT + quantization), alpha, HDR/wide-gamut, 10/12-bit and
4:2:0/4:2:2, image sequences, and the rest of the AV1/AVIF surface — is tracked, row by row
against the relevant specs, in [STATUS.md](STATUS.md).

## License

Licensed under either of MIT or Apache-2.0 at your option.
