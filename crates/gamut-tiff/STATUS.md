# gamut-tiff — TIFF 6.0 implementation status

Tracking GitHub issue #107: implement the **full TIFF 6.0 standard** (`references/tiff/tiff6.pdf`,
§1–23). Delivered as a stack of small, individually-reviewable PRs (P1–P19) onto the `feat/tiff`
integration branch; each PR is independently green (`mise run test`/`lint`/`fmt-check`/`coverage`
≥80%) and mergeable on its own.

**Keystone:** TIFF has no prediction/transform machinery — the hard part is the container
serialization spine (two-pass absolute-offset layout, ≤4-byte inline value packing, ascending-tag
sort, II/MM byte-order awareness). Once the uncompressed strip pipeline is **pixel-exact both
directions vs libtiff** (P3, gated by P4), each later phase just swaps a strip codec or photometric
mapping into the same spine.

**Oracle:** differential vs **libtiff** (dev-only FFI; `tooling/libtiff-oracle` +
`third_party/libtiff`). Lossless paths must agree **pixel-for-pixel** both directions (TIFF
permits many valid byte layouts, so the gate is pixel-exact, not byte-exact); JPEG-in-TIFF is
lossy → MAE/PSNR tolerance.

## Phases

| Phase | Spec § | Scope | Status |
| ----- | ------ | ----- | ------ |
| P1  | —       | Scaffold: crate, workspace wiring, docs, region-free skeleton | ✅ done |
| P2  | §2      | TIFF structure: header, IFD read/write, field types, value/offset packing | ✅ done |
| P3  | §3–4,6  | **Keystone** — uncompressed grayscale + RGB via strips; `Encoder`/`Decoder` | ✅ done |
| P4  | —       | libtiff oracle + pixel-exact both-direction differential gate | ✅ done |
| P5  | §9      | PackBits compression (8-bit gray/RGB) | ✅ done |
| P5b | §3      | Bilevel (1-bit) + FillOrder (4-bit gray deferred to P13) | ✅ done |
| P6  | §5      | Palette-color (ColorMap, 8-bit indices) | ✅ done |
| P7  | §7–8    | CLI `convert → .tiff` (uncompressed/PackBits RGB) | ✅ done |
| P8  | §10     | Modified Huffman (Compression=2) | ✅ done |
| P9  | §13     | LZW (Compression=5) | ✅ done |
| P10 | §14     | Differencing predictor (Predictor=2) | ✅ done |
| P11 | §11     | CCITT Group 4 / T.6 fax (Compression=4); G3-2D deferred | ✅ done |
| P12 | §15     | Tiled images (8-bit; None/PackBits/LZW) | ✅ done |
| P13 | §18     | RGBA (ExtraSamples alpha); planar / 16-bit / float deferred | ✅ done |
| P14 | §16     | CMYK (Separated, 8-bit) | ✅ done |
| P15 | §21     | YCbCr | ⏳ deferred |
| P16 | §20,23  | RGB colorimetry + CIE L\*a\*b\* | ⏳ deferred |
| P17 | §12     | Multi-page documents (halftone hints deferred) | ✅ done |
| P18 | §22     | JPEG-in-TIFF (Compression=7) — deferrable tail | ⏳ deferred |
| P19 | —       | Finalization: decoder robustness corpus + docs | ✅ done |

## Scope & dispositions (v1)

**Implemented (v1.0).** The full strip/tile serialization spine (two-pass absolute-offset layout,
II/MM, classic + BigTIFF), single- and multi-page documents, the 8-bit colour modes
(grayscale/RGB/RGBA/CMYK/palette) plus 1-bit bilevel, the compression schemes
None/PackBits/LZW (+ horizontal-differencing predictor)/Modified Huffman/Group 4 fax, and the
strict byte-accounting `deconstruct`. Evidence: every lossless path is pinned **pixel-exact in
both directions against libtiff** (`tests/oracle.rs` and the per-scheme differential suites over
`tooling/libtiff-oracle`), the decoder is fuzz-hardened over a byte-flip robustness corpus
(`tests/robustness.rs`), and the container spine is additionally covered by `gamut-ifd`'s own v1
oracle suite.

**Deferred (planned, additive).** Each plugs into the existing strip/tile pipeline and libtiff
oracle the way every codec above did:

- **YCbCr (§21, P15)** and **CIE L\*a\*b\* / RGB colorimetry (§20, §23, P16)** — need colour-space
  conversions in `gamut-color` matched bit-close to libtiff's integer math (cf. the WebP
  full-vs-limited-range trap), plus chroma subsampling for YCbCr.
- **New-style JPEG-in-TIFF (§22 as redefined by TIFF Technical Note 2, `Compression = 7`, P18)** —
  the DCT codec itself now exists in the workspace (`gamut-jpeg`, issue #28); the remaining work
  is the TN2 `JPEGTables`/strip wiring and a `libjpeg`-enabled libtiff oracle build.
- **Smaller deferrals:** CCITT Group 3 2-D / T.4 EOL framing (Group 3 1-D = the Modified Huffman of
  P8); `PlanarConfiguration = 2`; 16-bit / IEEE-float samples (§19) with the TN floating-point
  predictor 3; 4-bit grayscale; halftone hints (§17); document-storage metadata tags (§12 beyond
  `PageNumber`).

**Additivity guarantee:** each deferred row lands semver-minor — a new variant on a
`#[non_exhaustive]` enum (`Compression`, `PhotometricInterpretation`, `Predictor`), a new builder
method, or a new crate item — never a reshape of the frozen v1 surface.

**Permanently out of scope.** Old-style JPEG (`Compression = 6`, the original §22 scheme):
deprecated and unimplementable-as-specified per TIFF Technical Note 2, which replaced it wholesale
with `Compression = 7`. The `Compression::OldJpeg` variant exists only so the on-disk code
round-trips through `deconstruct`; neither encode nor decode will be implemented (maintainer
decision, issue #187).

**Permanently out of scope.** Pluggable codestream backends (the codestream-generation IoC seam
of #241): TIFF's compression schemes (None/PackBits/LZW/CCITT/Deflate) have no hardware
acceleration, so — unlike the hardware-accelerated formats that gain a backend seam
(PNG/JPEG/WebP/AVIF/HEIC/JXL) — gamut's own software implementation is always used and no
pluggable codestream backend is exposed. See AGENTS.md's convention on exposing the codestream
(maintainer decision, #241).

## v1 surface (issue #187)

The API was frozen after a full-surface review; the additions and breaks:

- **Single canonical paths** — the implementation modules are private; the surface is the
  crate-root re-export list (plus [`tags`](src/tags.rs), the one *named* module, mirroring
  `gamut_ifd::tags`). The per-scheme strip codecs (LZW/PackBits/CCITT/predictor) are
  crate-internal: every scheme is reachable through `Compression` on the encoder/decoder.
- **std conversions** — `Compression::{from_code, code}` and
  `PhotometricInterpretation::{from_code, code}` became `TryFrom<u32>` / `From<_> for u16`
  (the gamut-icc/gamut-isobmff precedent); `Predictor`, which had no conversions (inline `1|2`
  literals), gains the same symmetric pair.
- **`#[non_exhaustive]`** on the open code sets (`Compression`, `PhotometricInterpretation`,
  `Predictor`) and the grow-prone deconstruct types (`Severity`, `Anomaly` + its variants,
  `UnknownTag`, `DeconstructReport`), so new codes/categories/fields land semver-minor.
- **Complete re-export closure** — every gamut-ifd type reachable from this crate's public
  items is re-exported (since #263: `SegmentReport`, `Segment`, `SpanKind`, `DataLabel`,
  `Range`, `Conflict`, `SharedSpan`, `UnknownValue`, and `SubIfd`), so none needs a direct
  gamut-ifd dependency to name.
- **Documented freeze rationales** — `UnknownTag.field_type` stays a raw `u16` (unrecognised
  on-disk type codes must be representable); `Anomaly`'s `detail` strings are human-readable
  diagnostics whose wording is not contractual.
- **Dormant dependencies dropped** — `gamut-color` and `gamut-dsp` were declared but unused; they
  return additively with the colour-space and JPEG-in-TIFF work above.

## The v1 guarantee

`gamut-tiff` promises: every emitted file is a well-formed TIFF 6.0 (or BigTIFF) whose
structure the strict `deconstruct` fully classifies — since issue #263 with **zero tolerance**
(the byte-level verdict is `SegmentReport::is_fully_classified`: alignment padding comes back
as typed `Padding` segments, and the dual-ledger cross-check proves the parser's own
accounting); every lossless path round-trips pixel-exact and
is continuously validated in both directions against libtiff; the on-disk code↔enum mappings
(`Compression`, `PhotometricInterpretation`, `Predictor`) are frozen contracts; and every deferred
row above lands additively — the v1 public surface is never reshaped.
