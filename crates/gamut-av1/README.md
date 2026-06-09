# gamut-av1

`gamut-av1` is a pure-Rust AV1 image encoder. AVIF relies on AV1 intra-frame coding, so this crate
is usable standalone as well as through [`gamut-avif`](../gamut-avif).

If you want a complete `.avif` file, use [`gamut-avif`](../gamut-avif). Reach for this crate when you
need the **raw AV1 still bitstream** itself — to embed in your own container or build another
AV1-based format. It operates on `Planar8` planes and emits an AV1 temporal unit, not a container.

## Goals

Part of the [gamut](../../README.md) workspace, this crate exists to provide AV1 **still-image
encoding** that is:

- **Memory-safe on hostile input.** `#![forbid(unsafe_code)]` across the whole encode path, deleting
  the memory-corruption bug class that has repeatedly bitten the C AV1 codecs.
- **Clean-slate from the spec.** Implemented directly from the AV1 Bitstream & Decoding Process
  Specification ([`../../references/av1/`](../../references/av1)) rather than wrapping libaom, so it
  is auditable and forkable. Modules mirror the spec: `headers` = OBU framing + sequence/frame
  headers (§5.3/§5.5/§5.9), `tile` = partition/prediction/coefficient coding (§5.11), `cdf` =
  default CDF + scan + context tables (§9.2/§9.4/§8.3.2).
- **Decoder-verified.** Output is checked bit-exact against real decoders (`dav1d`, `libavif`),
  linked from vendored `third_party/` submodules — never from system-installed binaries.
- **Buildable anywhere `cargo` is.** No C, no nasm — cross-compiles cleanly (wasm32, aarch64, musl).

It builds on [`gamut-color`](../gamut-color) (pixel formats / CICP), [`gamut-dsp`](../gamut-dsp)
(the Walsh–Hadamard transform), and [`gamut-bitstream`](../gamut-bitstream) (bit writer + AV1 symbol
coder).

## Usage

```rust
use gamut_av1::encode_still_lossless_identity;
use gamut_color::Planar8;

// 8-bit interleaved RGB -> identity 4:4:4 planes -> lossless AV1 keyframe.
let (width, height) = (64u32, 64u32);
let rgb = vec![0u8; (width * height * 3) as usize];
let planes = Planar8::from_rgb8_identity(&rgb, width, height).expect("valid input");

let still = encode_still_lossless_identity(&planes).expect("encode");
// `still.obus` is the AV1 temporal unit; `still.config` carries the sequence-header
// values that `gamut-avif` mirrors into the `av1C`/`colr` boxes.
std::fs::write("out.obu", &still.obus).unwrap();
```

## Status

Today (milestone **M0**) the encoder implements a single, narrow path: a **lossless** all-intra
keyframe — `seq_profile = 1` (8-bit 4:4:4), identity matrix coefficients, full range, single tile,
64×64 superblocks, `DC_PRED`, and the forced `TX_4X4` Walsh–Hadamard transform, with static default
CDFs (`disable_cdf_update = 1`). It produces the AV1 temporal unit that `gamut-avif` wraps in an
AVIF still image.

The wider AV1 surface — lossy DCT/ADST, more intra modes, in-loop filters, inter coding for image
sequences — is tracked row by row in [`gamut-avif/STATUS.md`](../gamut-avif/STATUS.md).

## Roadmap

- M1: lossy intra (DCT/ADST + quantization), adaptive CDFs, more intra prediction modes.
- Later: in-loop filters, multi-tile, and inter coding for animated AVIF.

## License

Licensed under either of MIT or Apache-2.0 at your option.
