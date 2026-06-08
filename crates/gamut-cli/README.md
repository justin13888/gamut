# gamut-cli

`gamut-cli` ships the `gamut` binary — a command-line **sandbox** that exercises the workspace's
implemented primitives end to end, so the latest features are runnable from a shell without
writing throwaway Rust.

## Goals

Part of the [gamut](../../README.md) workspace, this crate exists to:

- **Make the encode pipeline runnable.** `gamut convert in.png out.avif` drives the full M0 path
  ([`gamut-color`](../gamut-color) → [`gamut-av1`](../gamut-av1) →
  [`gamut-isobmff`](../gamut-isobmff), surfaced through [`gamut-avif`](../gamut-avif)) on real
  image files.
- **Expose the shared primitives.** Each shared building block — color/CICP tables, the DSP
  Walsh–Hadamard transform, and the bitstream LEB128 coder — gets an inspection subcommand, so new
  primitives have an obvious place to be surfaced as they land.
- **Keep encoding pure gamut.** Input *decoding* borrows the third-party
  [`image`](https://crates.io/crates/image) crate (PNG/JPEG/PPM) for convenience, but everything
  *encoded* is produced by the gamut crates — the memory-safe, `#![forbid(unsafe_code)]` encode
  path stays intact.

The crate is `gamut-cli` (so `cargo install gamut-cli`), but it installs a binary named `gamut`.

## Usage

```bash
# Decode PNG/JPEG/PPM and encode AVIF (output format inferred from the extension).
gamut convert input.png output.avif

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

Today (milestone **M0**) the sandbox exposes:

- `convert` and `av1 encode` — **lossless** AVIF / raw AV1 still images from 8-bit RGB input.
- `color list`, `dsp wht`, `bitstream leb128` — inspection of the shared primitives.

Input is decoded with the `image` crate (PNG/JPEG/PPM); output is encoded only with gamut crates.
There is **no decoder** in the workspace yet, so round-trip verification is external — validate
AVIF output with `avifdec` and OBU output with `dav1d`. Only `avif` is a valid output format; the
other gamut format crates are still stubs and `convert` reports a clear error for them.

## Roadmap

As the codecs grow, so does the sandbox: lossy `convert` once AV1 gains DCT/quantization, more
output formats as `gamut-webp`/`gamut-jxl`/etc. fill in, an AVIF/WebP `info`/decode command once a
decoder exists, and a subcommand for each new primitive (e.g. the `gamut-bitstream` symbol coder).

## License

Licensed under either of MIT or Apache-2.0 at your option.
