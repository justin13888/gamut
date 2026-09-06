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

Everything here is produced by `cargo bench -p gamut-png`. What is *gated* is narrower, and
worth being precise about: `tests/size_contract.rs` asserts the size table -- every row including
`tiny_rgb8` and both `+clean` columns -- as a ratio against libpng-9 at 128×128, and pins
`with_transparent_cleanup` never costing bytes on any row. The throughput and per-heuristic tables
below are **reported, not gated**: timings cannot fail a build without making it flaky, which is
why CI runs the benches for compile-rot only ([#437]). One machine, so **read the ratios, not the
absolute times**.

### Output size vs libpng at zlib level 9

256×256 unless noted, gamut at `Level::Best` + `FilterStrategy::BruteForce` + auto-reduce.
`+clean` additionally enables `with_transparent_cleanup`. Lower is better.

| input | raw | default | best | +clean | libpng-9 | best/lp9 | bpp |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `gradient_rgb8` | 196 608 | 2 831 | 1 562 | 1 562 | 2 393 | **−34.7%** | 0.191 |
| `photo_rgb8` | 196 608 | 29 885 | 19 570 | 19 570 | 27 467 | **−28.8%** | 2.389 |
| `noise_rgb8` | 196 608 | 196 983 | 196 983 | 196 983 | 197 280 | −0.2% | 24.046 |
| `grey_as_rgb8` | 196 608 | 721 | 368 | 368 | 566 | **−35.0%** | 0.045 |
| `palette64_rgba8` | 262 144 | 1 274 | 726 | 688 | 1 102 | **−34.1%** | 0.089 |
| `sprite_rgba8` | 262 144 | 4 181 | 3 729 | **2 235** | 3 889 | −4.1% | 0.455 |
| `flat_rgba8` | 262 144 | 821 | 103 | 103 | 664 | **−84.5%** | 0.013 |
| `tiny_rgb8` (16×16) | 768 | 136 | 119 | 119 | 138 | **−13.8%** | 3.719 |

gamut is smaller than libpng-9 on every row, though `noise_rgb8` is a 0.2% near-tie rather than a
win: incompressible input leaves both encoders emitting stored blocks, so that row's budget is the
one deliberately set above parity (1.02) and it is excluded from the win assertion. The margin is
thin where no reduction applies
(`gradient`, `tiny`) or nothing is compressible (`noise`), and large where a lawful
representation change is available that libpng does not attempt.

### Filter heuristics (issue #480)

`BruteForce` tries every whole-image strategy and keeps the smallest, so the size table above
cannot say *which* heuristic earned the win. IDAT bytes at `Level::Best`, each heuristic alone:

| input | MinSumAbs | Entropy | Bigrams | winner |
| --- | --- | --- | --- | --- |
| `gradient_rgb8` | 2 215 | 2 215 | **1 505** | Bigrams |
| `photo_rgb8` | 25 364 | 22 427 | **19 513** | Bigrams |
| `noise_rgb8` | 196 890 | 196 890 | 196 890 | tie |
| `grey_as_rgb8` | **475** | 506 | 506 | MinSumAbs |
| `palette64_rgba8` | 990 | 899 | **770** | Bigrams |
| `sprite_rgba8` | **3 672** | 3 857 | 4 062 | MinSumAbs |
| `flat_rgba8` | **573** | 573 | 605 | MinSumAbs |
| `tiny_rgb8` | 79 | 79 | **62** | Bigrams |

**Bigrams wins four rows by 22–32%; MinSumAbs wins three by 5–6%.** Both stay in the brute-force
set: neither dominates, and the margins run the wrong way to drop either. That matches oxipng
keeping MinSum at `-o 0`/`-o 6` while its default preset leads with Bigrams.

**Entropy is never the unique winner**, and that is a recorded negative result. It beats MinSumAbs
on the photographic and palette rows but loses to Bigrams on both, and ties MinSumAbs elsewhere.
Since the brute-force set is resolved by taking the smallest, a candidate dominated everywhere
costs a full filter pass and a full DEFLATE for nothing — so it is not in that set. It stays
selectable: eight images is a corpus, not a proof.

### Throughput

| stage | before | after | |
| --- | --- | --- | --- |
| `crc32` | 420.8 MB/s | 8.996 GB/s | 21× |
| `filter_image` / None | 497.9 MB/s | 16.26 GB/s | 33× |
| `filter_image` / `Fixed(Paeth)` | 277.1 MB/s | 1.202 GB/s | 4.3× |
| `filter_image` / `MinSumAbs` | 46.7 MB/s | 265.8 MB/s | 5.7× |

All safe Rust: `crc32fast` keeps its `unsafe` to itself, and the filter gains are structural
(hoisting a loop-invariant branch, equal-length subslices, one `match` per row instead of per
byte) plus removing a sixth redundant filter pass per scanline.

### Per-axis state

| # | Axis | State |
| --- | --- | --- |
| 1 | Filter selection | **partial** — MinSumAbs, Entropy and Bigrams per line, plus seven whole-image candidates each fully DEFLATEd. Bigrams is worth 22–32% where it wins (see above). Still missing: per-line trial deflate, `AtomicMin` pruning, and a two-tier cheap-trial codec. [#480]. `FilterStrategy` became `#[non_exhaustive]` with this phase — a heuristic is a measurement result and the set grows with the corpus — which is a **breaking change** for any downstream exhaustive `match`: add a wildcard arm. |
| 2 | DEFLATE quality | **good, ~2% behind zopfli**, and honestly documented in `gamut-deflate`. Two contained wins remain: an 8-byte-at-a-time match compare, and `parse_dp`'s single-distance relaxation. [#478], [#479] |
| 3 | Smallest lawful representation | **done** — grey, alpha-drop, ≤256 palette, 16→8, sub-byte, and a `tRNS` colour key for grey/truecolour. The key is worth ~7–9% on a contiguous transparent region, *not* the 25% the raw-byte arithmetic suggests: the alpha plane it removes is usually the most compressible plane in the image. |
| 4 | Palette optimization | **partial** — trailing-opaque `tRNS` trim, plus ordering: transparent entries first (so that trim cuts as far as §11.3.2.1 allows) then by luma. Worth −14.7% on the sprite row against +1.5% on `palette64`. Modified-Zeng ordering and caller-supplied palette cleanup remain. [#482] |
| 5 | Cleaning invisible data | **done** — `with_transparent_cleanup`, opt-in, on every alpha-carrying layout at 8 and 16 bits. It is the crate's **one lossy knob**: it rewrites stored samples no decoder renders, where every other reduction here is byte-exact, which is why it is off by default and separate from `with_auto_reduce`. Worth **40.1%** on the sprite row, and it is what makes a colour key reachable at all on a source whose invisible pixels carry different unseen colours. It is a *transform*, not a reduction, so it is **raced** rather than assumed: on `palette64_rgba8` cleaning measured −2.3% at 32×32, **+10.7% at 128×128** and −5.2% at 256×256, because zeroing invisible pixels that carry structure destroys bytes DEFLATE was compressing. `cleaned_or_plain` encodes both and keeps the smaller, so the knob can never cost bytes. |
| 6 | Metadata hygiene | **no policy** — the encoder emits exactly what the caller set, and `gamut convert` drops metadata on the PNG path. The one exception is shape, not policy: `bKGD` and `sBIT` are resolved against the header actually written (see [Chunks that follow the race](#the-cost-model-and-why-it-is-a-race)). [#483] |
| 7 | Interlacing | **correctly none.** Adam7 costs 5–20%; out of scope by declaration. |
| 8 | Effort / speed / determinism | Output is byte-reproducible (no time, no randomness, and the one `HashMap` is never iterated). Three independent knobs, no composed dial. No parallelism. [#484] |
| 9 | Correctness / robustness | **covered** — 16-bit, odd dimensions, 1×1, CRC policy, malformed input. |

### The cost model, and why it is a race

`reduce::analyze8` chooses by comparing **raw** sizes, which does not predict compressed size when
one candidate's bytes are incompressible and the other's are not. A palette carries a `PLTE` (and
often `tRNS`) that DEFLATE cannot touch, while the pixels it replaces may compress by two orders of
magnitude. Measured on `palette64_rgba8`, whose palette candidate carries a flat 224 bytes of
`PLTE` + `tRNS` (192 + 8 payload, 24 framing) at every size — the fixture's colour count does not
depend on its side:

| side | emitted | IDAT | PLTE+tRNS emitted | libpng-9 |
| --- | --- | --- | --- | --- |
| 128 | 364 | 307 | — palette declined | 405 |
| 160 | 465 | 408 | — palette declined | 572 |
| 192 | 563 | 506 | — palette declined | 707 |
| 256 | 726 | 445 | 224 | 1 102 |

The raw-size estimate sees 16 664 against 65 536 and picks the palette by 4× **at every one of
these sizes**. The finished files disagree: the palette's 224 fixed bytes are incompressible while
the pixels they replace compress by two orders of magnitude, so indexing only pays once the image
is large enough to amortise them — the crossover sits between 192 and 256. So
`write_reduced_or_native` encodes both candidates and keeps the smaller, the same way
`FilterStrategy::BruteForce` already resolves filters — no tuned constant, and never worse than
either candidate alone. The three declined rows are the evidence: had the estimate been trusted,
each would have carried a palette and been larger. Only palette reductions pay for the second
encode; greyscale, alpha-drop and 16→8 demotion add no chunks, so for them the raw comparison is
sound.

**What the races cost.** Each race is a full extra encode, and they nest: `FilterStrategy::BruteForce`
tries seven whole-image strategies, `write_reduced_or_native` encodes both candidates when the
reduction carries a chunk (a palette's `PLTE`/`tRNS`, a colour key's `tRNS`), and `cleaned_or_plain`
encodes both the cleaned and the untouched samples when cleanup changed anything. The worst case —
`Level::Best` + `BruteForce` + auto-reduce + cleanup on an alpha image that is both cleanable and
palettisable or keyable — is therefore 7 × 2 × 2 = **28** filter-plus-DEFLATE passes for one file,
against 7 for `BruteForce` alone. That is the price of choosing by measured size rather than by a
cost model; a model good enough to skip the losing candidate is [#480]'s remainder.

**Chunks that follow the race.** `bKGD` and `sBIT` have a payload whose shape is the colour type, and
the race decides the colour type after they were set. Both are resolved against the header actually
written — RGBA `sBIT` loses its alpha entry under RGB or a palette, an RGB or grey background under a
palette becomes the index of its entry (an opaque entry where a transparent twin exists), a grey RGB
triple collapses to one grey sample — and omitted, without error, where no lossless conversion
exists, since a payload shaped for the wrong colour type is a chunk libpng rejects and drops. A
caller's palette *index* survives only on the `encode_indexed8` path, whose palette is the caller's;
under an encoder-derived palette it names nothing and is omitted. This holds across colour
**types**; on the depth axis a `bKGD` sample is range-checked but not rescaled with a 16→8 demotion
or a sub-byte packing — that is [#501].

[#437]: https://github.com/visualcommons/gamut/issues/437
[#478]: https://github.com/visualcommons/gamut/issues/478
[#479]: https://github.com/visualcommons/gamut/issues/479
[#480]: https://github.com/visualcommons/gamut/issues/480
[#481]: https://github.com/visualcommons/gamut/issues/481
[#482]: https://github.com/visualcommons/gamut/issues/482
[#483]: https://github.com/visualcommons/gamut/issues/483
[#484]: https://github.com/visualcommons/gamut/issues/484
[#501]: https://github.com/visualcommons/gamut/issues/501
