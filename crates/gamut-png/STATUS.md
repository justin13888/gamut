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
size is benchmarked against libpng at maximum compression.

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

## Decoder phases (issue #249)

| Phase | Spec | Scope | Status |
| ----- | ---- | ----- | ------ |
| D1 | §5, §11.2.1 | libpng-oracle reference *encode* entry point (fixture generator); chunk-stream parser + CRC policy; IHDR validation | ✅ done |
| D2 | §9, §10 | `PngDecoder` + decode limits (dimensions, byte budget); bounded zlib inflation (`miniz_oxide`); scanline defilter; non-interlaced typed `DecodeImage` matrix (lossless widening via `gamut_core::convert`) | ✅ done |
| D3 | §11.2.2/§11.3.1 | Palette + tRNS: `PngPalette::from_chunks`, index range checks, `DecodeImage<Indexed8>`, RGB(A) expansion, colour keys | ✅ done |
| D4 | §8.1, §13.10 | Adam7 de-interlacing (per-pass defilter/unpack, empty passes, checked stream-length sum) | ✅ done |
| D5 | §11.3 | Rich `decode()` → `DecodedPng`: raw eXIf/iCCP/XMP/text payloads (MetadataBlock-ready), parsed gAMA/cHRM/sRGB/cICP, metadata inflation budget | ✅ done |
| D6 | — | libpng differential conformance suite over generated fixtures; malformed-input rejection corpus; mutation-gap closure | ✅ done |
