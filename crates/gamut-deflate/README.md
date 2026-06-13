# gamut-deflate

`gamut-deflate` is a pure-Rust **DEFLATE** (RFC 1951) and **zlib** (RFC 1950) *encoder*.

## Goals

Part of the [gamut](../../README.md) workspace, this crate is the shared compression primitive
behind the codecs that embed DEFLATE streams — most directly [`gamut-png`](../gamut-png) (`IDAT`,
`zTXt`, `iCCP`), and reusable by TIFF (`Compression=8`) and any other zlib/gzip consumer. It is:

- **Encoder-only.** Inflating DEFLATE is a solved problem with strong implementations everywhere, so
  — per gamut's encoder-first philosophy — this crate does not decode. Correctness is proven
  differentially against the canonical C `zlib` (see Validation).
- **Space-efficient.** Encoding latency is secondary to ratio. [`Level::Best`] targets a
  zopfli-class optimal parse with package-merge length-limited Huffman codes, approaching the
  smallest streams the format allows.
- **Self-contained and safe.** `#![forbid(unsafe_code)]`, no internal dependencies.

## Usage

```rust
use gamut_deflate::{DeflateEncoder, Level};

let data = b"the quick brown fox jumps over the lazy dog".repeat(8);

// Raw DEFLATE (RFC 1951):
let mut raw = Vec::new();
DeflateEncoder::new().compress(&data, &mut raw);

// zlib-wrapped (RFC 1950) — what PNG's IDAT carries:
let mut zlib = Vec::new();
DeflateEncoder::new().with_level(Level::Best).zlib_compress(&data, &mut zlib);
```

## Status

Built incrementally; each phase is conformance-checked against zlib (see [STATUS.md](STATUS.md)).
The encoder always produces correct streams — phases progressively improve the ratio, from stored
blocks up through fixed/dynamic Huffman, block splitting, and a zopfli-style optimal parse.

## Validation

A differential oracle (`tooling/zlib-oracle`, a vendored static `zlib`) inflates the encoder's
output and asserts it round-trips to the original bytes, and benchmarks output size against zlib's
own compressor.

## License

Licensed under either of MIT or Apache-2.0 at your option.
