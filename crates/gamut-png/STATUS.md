# gamut-png — PNG codec status

Tracking GitHub issues #24 (encoder) and #249 (decoder): a research-grade, space-efficient PNG
**encoder** on par with the best PNG encoders, and a spec-compliant **decoder** for the rawshift
migration off `zune-png`. Delivered as small, individually green phases (each
`mise run test`/`lint`/`fmt-check`/`coverage` ≥80%).

**Keystone:** the signature → IHDR → IDAT → IEND pipeline with filter-None 8-bit RGB (P2) — once
libpng decodes that pixel-exact, each later phase swaps in another colour type, a filter, or a
space optimisation behind the same chunk spine and CRC.

**Oracle:** differential vs **libpng** (`tooling/libpng-oracle` + `third_party/libpng`, dev-only
FFI), in both directions: libpng decodes the encoder's output → pixel-exact with the source, and a
libpng reference-encode entry point generates the decoder's conformance fixtures (interlaced,
sub-byte, forced-filter, metadata-laden) that gamut-png and libpng must decode identically. Output
size is measured against libpng at zlib level 9 by `cargo bench -p gamut-png` and enforced by
`tests/size_contract.rs` (see [Efficiency](#efficiency-issue-224)).

**Out of scope:** Adam7 *encoding*, animation/APNG (gamut is image-first; the decoder reads an
APNG's default image). Format-agnostic pixel conversion (grey↔RGB, alpha, 16↔8-bit) is
[`gamut_core::convert`]'s (issue #268), not this crate's: the decoder resolves what only PNG knows —
palette lookup, folding a tRNS key into a real alpha channel, §13.12 sub-byte scaling — and hands the
layout change to the shared engine. A typed decode is lossless by default; `PngDecoder::convert_policy`
opts into narrowing. That is distinct from the encoder's *lossless* auto-reduce (#338), which demotes
16→8 only when every sample is exactly `k·257` and drops alpha only when fully opaque.

## Phases

| Phase | Spec | Scope | Status |
| ----- | ---- | ----- | ------ |
| P1 | §5, §11.2.1 | Scaffold + workspace wiring + libpng-oracle/submodule; CRC-32; chunk writer + signature; `ColorType` + bit-depth matrix; IHDR | ✅ done |
| P2 | §6, §9, §11.2.4 | **Keystone:** `EncodeImage<Rgb8>`, filter None, DEFLATE → signature/IHDR/IDAT/IEND | ✅ done |
| P3 | §9 | All 5 scanline filters (None/Sub/Up/Average/Paeth) + `MinSumAbs` selection | ✅ done |
| P4 | §6.1 | Colour types: Gray8/Gray16/Rgb16/Rgba8/Rgba16/GrayAlpha8/16 (16-bit big-endian) | ✅ done |
| P5 | §11.2.2/§11.3.2 | Indexed (`encode_indexed8` + PLTE + tRNS), 8-bit | ✅ done |
| P6 | §7.2 | Sub-byte depths: 1-bit bilevel grey + auto-minimal-depth indexed (1/2/4) | ✅ done |
| P7 | §11.3 | Standard ancillary chunks: gAMA/cHRM/sRGB/sBIT/bKGD/pHYs/tIME/tEXt/zTXt/iTXt | ✅ done |
| P8 | §11.3 | Metadata: eXIf, iCCP (deflate-compressed), iTXt-XMP (raw-bytes setters) | ✅ done |
| P9 | §4.5 | **Space opt:** lossless palette/gray/alpha-drop reduction (size-estimate chosen) + brute-force filter strategy; extended to grey/grey-alpha/16-bit inputs with lossless 16→8 demotion and sub-byte grey packing (#338) | ✅ done |
| P10 | — | CLI `gamut convert → .png`; umbrella `png` feature; final API review | ✅ done |
| E1 | #224 | **Efficiency:** `deconstruct` byte accounting; divan size/bpp + per-stage bench; libpng-9 size contract; opt-in transparent cleanup; palette-vs-native race; `crc32fast` and restructured filter kernels (see [Efficiency](#efficiency-issue-224)) | ✅ done |

## Decoder phases (issue #249)

| Phase | Spec | Scope | Status |
| ----- | ---- | ----- | ------ |
| D1 | §5, §11.2.1 | libpng-oracle reference *encode* entry point (fixture generator); chunk-stream parser + CRC policy; IHDR validation | ✅ done |
| D2 | §9, §10 | `PngDecoder` + decode limits (dimensions, byte budget); bounded zlib inflation (`miniz_oxide`); scanline defilter; non-interlaced typed `DecodeImage` matrix (lossless widening via `gamut_core::convert`) | ✅ done |
| D3 | §11.2.2/§11.3.1 | Palette + tRNS: `PngPalette::from_chunks`, index range checks, `DecodeImage<Indexed8>`, RGB(A) expansion, colour keys | ✅ done |
| D4 | §8.1, §13.10 | Adam7 de-interlacing (per-pass defilter/unpack, empty passes, checked stream-length sum) | ✅ done |
| D5 | §11.3 | Rich `decode()` → `DecodedPng`: raw eXIf/iCCP/XMP/text payloads (MetadataBlock-ready), parsed gAMA/cHRM/sRGB/cICP, metadata inflation budget | ✅ done |
| D6 | — | libpng differential conformance suite over generated fixtures; malformed-input rejection corpus; mutation-gap closure | ✅ done |
| D7 | §5, §11.3 | Pixel-free metadata entry point (issue #379): `metadata()` / `PngDecoder::metadata()` → `PngMetadata`, sharing one chunk-classification predicate with `decode()`; IDAT skipped by length, never read or inflated. Mirrors `gamut_jpeg::metadata` / `gamut_webp::metadata` | ✅ done |

## Efficiency (issue #224)

Correctness was settled long before efficiency was measured. This section is the measured state:
what the encoder achieves, what it costs, and — per axis — what it does not do yet.

Everything here is produced by `cargo bench -p gamut-png` and gated by
`tests/size_contract.rs`. One machine, so **read the ratios, not the absolute times**.

### Output size vs libpng at zlib level 9

256×256 unless noted, gamut at `Level::Best` + `FilterStrategy::BruteForce` + auto-reduce.
`+clean` additionally enables `with_transparent_cleanup`. Lower is better.

| input | raw | default | best | +clean | libpng-9 | best/lp9 | bpp |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `gradient_rgb8` | 196 608 | 2 831 | 2 272 | 2 272 | 2 393 | **−5.1%** | 0.277 |
| `photo_rgb8` | 196 608 | 29 885 | 20 293 | 20 293 | 27 467 | **−26.1%** | 2.477 |
| `noise_rgb8` | 196 608 | 196 983 | 196 983 | 196 983 | 197 280 | −0.2% | 24.046 |
| `grey_as_rgb8` | 196 608 | 721 | 370 | 370 | 566 | **−34.6%** | 0.045 |
| `palette64_rgba8` | 262 144 | 1 274 | 715 | 682 | 1 102 | **−35.1%** | 0.087 |
| `sprite_rgba8` | 262 144 | 4 181 | 3 729 | **2 619** | 3 889 | −4.1% | 0.455 |
| `flat_rgba8` | 262 144 | 821 | 103 | 103 | 664 | **−84.5%** | 0.013 |
| `tiny_rgb8` (16×16) | 768 | 136 | 135 | 135 | 138 | −2.2% | 4.219 |

gamut is smaller than libpng-9 on every row. The margin is thin where no reduction applies
(`gradient`, `tiny`) or nothing is compressible (`noise`), and large where a lawful
representation change is available that libpng does not attempt.

### Throughput

| stage | before | after | |
| --- | --- | --- | --- |
| `crc32` | 420.8 MB/s | 8.996 GB/s | 21× |
| `filter_image` / None | 497.9 MB/s | 16.26 GB/s | 33× |
| `filter_image` / `Fixed(Paeth)` | 277.1 MB/s | 1.202 GB/s | 4.3× |
| `filter_image` / `MinSumAbs` | 46.7 MB/s | 265.8 MB/s | 5.7× |
| `choose_min_sum_abs` | 68.0 MB/s | 308.4 MB/s | 4.5× |

All safe Rust: `crc32fast` keeps its `unsafe` to itself, and the filter gains are structural
(hoisting a loop-invariant branch, equal-length subslices, one `match` per row instead of per
byte) plus removing a sixth redundant filter pass per scanline.

### Per-axis state

| # | Axis | State |
| --- | --- | --- |
| 1 | Filter selection | **partial** — per-line MinSumAbs plus six whole-image candidates each fully DEFLATEd. No entropy or bigram heuristic, no per-line trial deflate, no pruning, no two-tier trial. [#480] |
| 2 | DEFLATE quality | **good, ~2% behind zopfli**, and honestly documented in `gamut-deflate`. Two contained wins remain: an 8-byte-at-a-time match compare, and `parse_dp`'s single-distance relaxation. [#478], [#479] |
| 3 | Smallest lawful representation | **partial** — grey, alpha-drop, ≤256 palette, 16→8, sub-byte all present; a `tRNS` colour key for grey/truecolour is not. [#481] |
| 4 | Palette optimization | **minimal** — trailing-opaque `tRNS` trim only. First-appearance order, no sorting; caller-supplied palettes get no dedupe or unused-entry removal. [#482] |
| 5 | Cleaning invisible data | **done** — `with_transparent_cleanup`, opt-in. Worth 30% on the sprite row. |
| 6 | Metadata hygiene | **no policy** — the encoder emits exactly what the caller set, and `gamut convert` drops metadata on the PNG path. [#483] |
| 7 | Interlacing | **correctly none.** Adam7 costs 5–20%; out of scope by declaration. |
| 8 | Effort / speed / determinism | Output is byte-reproducible (no time, no randomness, and the one `HashMap` is never iterated). Three independent knobs, no composed dial. No parallelism. [#484] |
| 9 | Correctness / robustness | **covered** — 16-bit, odd dimensions, 1×1, CRC policy, malformed input. |

### The cost model, and why it is a race

`reduce::analyze8` chooses by comparing **raw** sizes, which does not predict compressed size when
one candidate's bytes are incompressible and the other's are not. A palette carries a `PLTE` (and
often `tRNS`) that DEFLATE cannot touch, while the pixels it replaces may compress by two orders of
magnitude. Measured on `palette64_rgba8`, where `PLTE` + `tRNS` is a flat 273 bytes:

| side | gamut | IDAT | PLTE+tRNS | libpng-9 |
| --- | --- | --- | --- | --- |
| 128 | 451 | 121 | 273 | 405 |
| 160 | 511 | 181 | 273 | 572 |
| 192 | 564 | 234 | 273 | 707 |
| 256 | 715 | 385 | 273 | 1 102 |

The estimate sees 16 664 against 65 536 and picks the palette by 4×; the finished files cross over
near 160×160. So `write_reduced_or_native` encodes both candidates and keeps the smaller, the same
way `FilterStrategy::BruteForce` already resolves filters — no tuned constant, and never worse than
either candidate alone. Only palette reductions pay for the second encode; greyscale, alpha-drop
and 16→8 demotion add no chunks, so for them the raw comparison is sound.

[#478]: https://github.com/visualcommons/gamut/issues/478
[#479]: https://github.com/visualcommons/gamut/issues/479
[#480]: https://github.com/visualcommons/gamut/issues/480
[#481]: https://github.com/visualcommons/gamut/issues/481
[#482]: https://github.com/visualcommons/gamut/issues/482
[#483]: https://github.com/visualcommons/gamut/issues/483
[#484]: https://github.com/visualcommons/gamut/issues/484
