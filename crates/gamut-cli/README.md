# gamut-cli

`gamut-cli` ships the `gamut` binary — a command-line **sandbox** that exercises the workspace's
implemented primitives end to end, so the latest features are runnable from a shell without
writing throwaway Rust.

## Goals

Part of the [gamut](../../README.md) workspace, this crate exists to:

- **Make the codec pipelines runnable.** `gamut convert in.png out.avif` drives the AVIF encode
  path ([`gamut-color`](../gamut-color) → [`gamut-av1`](../gamut-av1) →
  [`gamut-isobmff`](../gamut-isobmff), surfaced through [`gamut-avif`](../gamut-avif)); `gamut
  convert in.png out.webp` drives [`gamut-webp`](../gamut-webp) (VP8L lossless / VP8 lossy, with
  alpha). WebP can also be read back — `gamut convert in.webp out.avif` decodes it through gamut's
  own WebP decoder — so the encode→decode round-trip is exercisable end to end.
- **Expose the shared primitives.** Each shared building block — color/CICP tables, the DSP
  Walsh–Hadamard transform, and the bitstream LEB128 coder — gets an inspection subcommand, so new
  primitives have an obvious place to be surfaced as they land.
- **Keep the codec path pure gamut.** *Encoding* is always produced by the gamut crates. *Decoding*
  of PNG/JPEG/PPM inputs borrows the third-party [`image`](https://crates.io/crates/image) crate for
  convenience, but **WebP input is decoded by gamut's own decoder** (no third-party webp library) —
  so the full WebP path, both directions, stays on the memory-safe, `#![forbid(unsafe_code)]` gamut
  code.

The crate is `gamut-cli` (so `cargo install gamut-cli`), but it installs a binary named `gamut`.

## Usage

```bash
# Decode PNG/JPEG/PPM/WebP and encode AVIF (output format inferred from the extension).
gamut convert input.png output.avif

# Encode WebP: lossless VP8L by default, or lossy VP8 with --lossy (transparency is preserved).
gamut convert input.png output.webp
gamut convert input.png output.webp --lossy --quality 80

# Read WebP back and transcode it — decoded by gamut's own WebP decoder, no third-party lib.
gamut convert output.webp roundtrip.avif

# Encode a raw AV1 OBU temporal unit you can hand to a decoder.
gamut av1 encode input.ppm output.obu
dav1d -i output.obu -o roundtrip.y4m      # external check

# Inspect the gamut-color CICP / pixel-format tables.
gamut color list

# Run the 4x4 Walsh–Hadamard transform over 16 ints and verify the round-trip.
gamut dsp wht 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0

# Show the unsigned LEB128 encoding of a value.
gamut bitstream leb128 300                # -> ac 02 (2 bytes)

# Logging goes to stderr; -v = info, -vv = debug, or set RUST_LOG.
gamut -vv convert input.jpg output.avif
```

## Status

The sandbox exposes:

- `convert` — decode PNG/JPEG/PPM/WebP and encode to a gamut codec:
  - **AVIF** — lossless (default) or lossy intra via `--qindex` (8-bit RGB).
  - **WebP** — lossless VP8L (default) or lossy VP8 via `--lossy --quality`, with transparency
    preserved; emits a simple file when fully opaque and an extended (`VP8X`/`ALPH`) file otherwise.
- `av1 encode` — raw AV1 OBU still images from 8-bit RGB input.
- `color list`, `dsp wht`, `bitstream leb128` — inspection of the shared primitives.

Output is always encoded by gamut crates. Input decoding uses the `image` crate for PNG/JPEG/PPM,
while **WebP input is decoded by gamut's own decoder** — so a WebP round-trip (`png → webp → avif`)
runs entirely in-tool. AVIF/AV1 output still has no in-workspace decoder, so verify it externally
(`avifdec` / `dav1d`). `avif` and `webp` are the supported output formats; `convert` reports a clear
error for anything else.

## Roadmap

As the codecs grow, so does the sandbox: an in-tool AVIF/AV1 decode path once a gamut AV1 decoder
exists (WebP already decodes), an explicit `info`/decode-to-pixels command, more output formats as
`gamut-jxl`/etc. fill in, and a subcommand for each new primitive (e.g. the `gamut-bitstream` symbol
coder).

## License

Licensed under either of MIT or Apache-2.0 at your option.
