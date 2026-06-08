# gamut

`gamut` is the umbrella crate for a collection of space-efficient image encoding libraries. It
re-exports the format-specific crates behind Cargo features, so a consumer compiles only the codecs
they need.

For the full "why gamut" rationale (memory safety, clean license story, WASM-first, encoder-first),
see the [workspace README](../../README.md).

## Goals

Part of the [gamut](../../README.md) workspace, this crate exists to be the **single entry point**
consumers depend on:

- **One dependency, opt-in codecs.** Add `gamut` and turn on only the formats you use; everything is
  feature-gated and `default = []`, so a bare dependency compiles just [`gamut-core`](../gamut-core).
- **A consistent API across formats.** [`gamut::core`] is always available and provides the shared
  [`Encoder`]/[`Decoder`] traits and the common `Error` type, regardless of which formats are
  enabled.
- **Tooling access to the primitives.** The `primitives` feature additionally re-exports the shared
  building blocks — `gamut::color`, `gamut::dsp`, `gamut::bitstream` — for inspection and sandbox use
  (this is what the [`gamut` CLI](../gamut-cli) is built on).

## Features

`default = []` — a bare dependency compiles only `gamut-core`.

| Feature      | Enables                                                                 |
| ------------ | ----------------------------------------------------------------------- |
| `avif`       | AVIF encoding (`gamut-avif` + `gamut-av1`)                               |
| `av1`        | Standalone AV1 image encoding (`gamut-av1`)                             |
| `webp`       | WebP (`gamut-webp`) — placeholder                                       |
| `jxl`        | JPEG XL (`gamut-jxl`) — placeholder                                     |
| `heic`       | HEIC/HEIF (`gamut-heic`) — placeholder                                  |
| `vvc`        | VVC / H.266 (`gamut-vvc`) — placeholder                                 |
| `av2`        | AV2 (`gamut-av2`) — placeholder                                         |
| `primitives` | Re-export `color` / `dsp` / `bitstream` for tooling                     |
| `all`        | Every format above **plus** `primitives`                                |

## Usage

```toml
[dependencies]
gamut = { version = "0.1", features = ["avif"] }
```

```rust
use gamut::avif::AvifEncoder;
use gamut::core::{Dimensions, Encoder};

let (width, height) = (64usize, 64usize);
let rgb = vec![0u8; width * height * 3]; // 8-bit interleaved RGB

let mut out = Vec::new();
AvifEncoder::new()
    .encode_rgb8(&rgb, Dimensions { width: width as u32, height: height as u32 }, &mut out)
    .expect("encode");
```

## Status

`avif` and `av1` are functional for M0 lossless still images; the other format features currently
gate placeholder crates that return `Unsupported`. The implemented surface tracks
[`gamut-avif/STATUS.md`](../gamut-avif/STATUS.md). See the [workspace README](../../README.md) for the
per-crate status table.

## License

Licensed under either of MIT or Apache-2.0 at your option.
