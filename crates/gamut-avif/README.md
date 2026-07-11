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
use gamut_core::{Dimensions, EncodeImage, ImageRef, Rgb8};

let (width, height) = (64u32, 64);
let rgb: Vec<u8> = vec![0; (width * height * 3) as usize]; // 8-bit interleaved RGB
let dims = Dimensions { width, height };
let image = ImageRef::<Rgb8>::new(&rgb, dims).expect("buffer length matches dimensions");

// Lossless by default. `AvifEncoder::lossy(quality)` (0..=100) trades fidelity for a smaller
// file; `with_rotation` / `with_mirror` add display-orientation transforms.
let avif = AvifEncoder::new().encode_to_vec(image).expect("encode");
std::fs::write("out.avif", &avif).unwrap();
```

`AvifEncoder` implements [`gamut_core::EncodeImage<Rgb8>`], so the input is a typed
[`gamut_core::ImageRef`] and handing it an unsupported pixel layout is a compile error.

## Status

**v1 surface.** The encoder produces **lossless** (the default) and **lossy**
(`AvifEncoder::lossy(quality)`) still images: 8-bit RGB mapped to AV1 identity-matrix 4:4:4 and
wrapped as a single `av01` item in a conformant MIAF/AVIF container. Lossless output is bit-exact
to the input; lossy trades fidelity for size on a `0..=100` quality scale (higher = closer to the
source; the `quality → base_q_idx` mapping and its silent clamp above 100 are a frozen v1
contract, defined in [`references/avif`](../../references/avif/README.md)). `irot`/`imir` display
orientation is supported. Output is verified against real decoders (`libavif`, `dav1d`, `libaom`),
linked from vendored `third_party/` submodules rather than system-installed binaries.

Everything beyond is dispositioned in [STATUS.md](STATUS.md), row by row against the relevant
specs: **deferred, planned** features (alpha, HDR/wide-gamut, 10/12-bit and 4:2:0/4:2:2,
ICC/Exif/XMP metadata, gain maps, layered/progressive images, an AVIF decoder, …) all land
semver-minor on the frozen v1 surface, while image sequences/tracks and AV1 inter coding are
**permanently out of scope** per the image-first workspace charter.

## License

Licensed under either of MIT or Apache-2.0 at your option.
