# DEFLATE / zlib references

Specifications implemented by [`gamut-deflate`](../../crates/gamut-deflate) — a pure-Rust,
space-optimizing DEFLATE **encoder**.

## Vendored primary sources

- **RFC 1951** — *DEFLATE Compressed Data Format Specification v1.3*: `rfc1951.txt`
- **RFC 1950** — *ZLIB Compressed Data Format Specification v3.3* (the header/Adler-32 wrapper):
  `rfc1950.txt`

(The same two RFCs are vendored under [`../png`](../png) because PNG's `IDAT`/`zTXt`/`iCCP` carry
zlib streams; `gamut-deflate` is their primary implementer, so canonical copies live here too.)

## Spec → implementation map

Every construct the encoder emits is traceable to an RFC section:

| RFC 1951 section | Construct | Where |
| --- | --- | --- |
| §3.1.1 | LSB-first bit packing | `src/bitwriter.rs` |
| §3.2.2 | Huffman code construction from code lengths | `src/huffman.rs` |
| §3.2.3 | Block format / `BFINAL`,`BTYPE` | `src/block.rs`, `src/dynamic.rs` |
| §3.2.4 | Stored (non-compressed) blocks | `src/block.rs` |
| §3.2.5 | Length/distance codes + extra bits | `src/symbols.rs` |
| §3.2.6 | Fixed Huffman codes | `src/huffman.rs`, `src/block.rs` |
| §3.2.7 | Dynamic Huffman codes + code-length RLE (16/17/18) | `src/dynamic.rs` |
| §4 | LZ77 matching (`MIN_MATCH`..`MAX_MATCH`, 32 KiB window) | `src/lz77.rs` |

| RFC 1950 section | Construct | Where |
| --- | --- | --- |
| §2.2 | zlib header (CMF/FLG, FCHECK, FLEVEL) | `src/zlib.rs` |
| §9 | Adler-32 checksum | `src/adler32.rs` |

## Oracle

Correctness is proven **differentially** against the canonical C **zlib** (v1.3.1), built from the
`third_party/zlib` submodule and exposed through the dev-only `tooling/zlib-oracle` FFI crate. The
gate: inflate the encoder's output with zlib and assert it round-trips byte-exact to the input, for
every level, across an edge-case corpus and a randomized property test. The oracle also supplies the
`zlib -9` size baseline for the ratio contract (`Level::Best ≤ zlib -9`).

## Scope & rationale

- **Encoder only.** Inflating DEFLATE is a solved, security-sensitive problem; gamut's decoders that
  need it use the safe, fuzzed `miniz_oxide` rather than a fresh inflater. Per gamut's
  encoder-first philosophy.
- **Space-first.** The crate exists to beat general-purpose encoders on ratio: `Level::Best` (a
  zopfli-style optimal parse) produces smaller streams than both `zlib -9` and `miniz_oxide` at max
  effort, staying within a few percent of the `zopfli` crate. See the crate `README.md` for numbers.
- **Out of scope:** gzip framing (RFC 1952) and its CRC-32 — no gamut image format uses gzip; PNG's
  chunk CRC-32 (a different polynomial) lives in `gamut-png`.
