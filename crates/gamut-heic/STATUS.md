# gamut-heic — HEIC/HEIF decoder implementation status

The component surface a conformant HEIF/HEIC still-image **decoder** needs, drawn from every related
spec (ISO/IEC 23008-12 HEIF; ISO/IEC 23000-22 MIAF; ISO/IEC 14496-12 ISOBMFF; ISO/IEC 14496-15 NAL
file format; ITU-T H.265 | ISO/IEC 23008-2 HEVC; ITU-T H.273 CICP). Rows are **technical
components**, not user features. gamut is **decode-only** for HEIF — there is no encoder row.

**Status:** ✅ = implemented · ☐ = deferred (planned, additive, in a named later slice)
· **OOS** = permanently out of scope. The **Slice** column names the delivery: **S1** = the container
parsing + byte accounting + role-typed view slice (issue #238); **S2** = the `hvcC` record +
NAL demux/classification slice (delivered — `src/hvcc.rs`, `src/nal.rs`); **S3** = the decoder trait +
HEVC-intra pixel pipeline slice; **S4** = the libheif differential-oracle slice.

This crate builds on [`gamut-isobmff` v1](../gamut-isobmff/STATUS.md): the box grammar, item model,
property/reference parsing, and motion-photo *tolerance* already ship there. This ledger mirrors
that finalized disposition rather than contradicting it — a ✅ here is the HEIF-specific layer
(byte accounting, role typing, primary validation) on top of that shared container.

## Scope & dispositions

**Implemented (S1).** `HeifContainer::parse` gives the *total* byte-accounting representation — a
contiguous, non-overlapping segment list covering every byte of the file (top-level boxes including
unrecognised ones like `mpvd`, an appended foreign stream from a second `ftyp`, or an explicit
trailer), with `meta`/`iprp` shadow-walked for unconsumed boxes — plus `HeifImage`, the role-typed
semantic view over the primary still-image stream (brands, validated primary, item kinds, typed
properties, and the thumbnail/auxiliary/metadata/derivation relationship lenses). The coded
bitstream stays opaque.

**Implemented (S2).** `HevcConfig::parse` decodes the `hvcC` HEVCDecoderConfigurationRecord (§1) into
typed fields plus the parameter-set arrays, reached from a coded item via `HeifItem::hevc_config`;
`iter_nal_units` splits a length-prefixed `hvc1`/`hev1` payload (§2); `NalUnitType`/`NalHeader`
classify each NAL unit (§3); `HevcConfig::validate_still_payload` enforces the still-image IRAP
constraint and `HevcConfig::annex_b` emits a start-coded stream for a downstream decoder. Still
parsing/classification only — the RBSP payloads stay opaque (decode is S3 / issue #18).

**Deferred (planned, additive).** The decoder trait + HEVC-intra pixel pipeline and the libheif
oracle (rows below). Each lands additively — new crate items or new `#[non_exhaustive]` variants —
never a reshape of the shipped surface.

**Permanently out of scope** (workspace charter: image-first, no inter-frame/motion/sequence
coding). Image sequences and tracks — the `msf1`/`hevc`/`hevx` brands, `moov`/`trak`/`mdia`/`stbl`
and `hvc1` sample entries — and all HEVC inter-coding; L-HEVC multi-layer *decode* (layered brands
`heim`/`heis` are read as their single base layer); protected/`uri ` items; external data
references (`dinf`/`dref`, `iloc` `construction_method` 2); mirroring the finalized
[`gamut-isobmff` ledger](../gamut-isobmff/STATUS.md).

## A. Container / byte accounting (23008-12 · MIAF · 14496-12)

| Component | Spec | Status | Slice |
| --- | --- | --- | --- |
| Total representation: contiguous `segments` covering `0..len` (the every-byte invariant) | #238 | ✅ | S1 |
| Top-level box walk (shared `gamut_isobmff::BoxReader`), unknown boxes surfaced verbatim | 14496-12 §4.2 | ✅ | S1 |
| Unknown top-level box (e.g. Google `mpvd` Motion Photo Video Data) retained as `SegmentKind::Box` | Android Motion Photo; `references/heif` §8a | ✅ | S1 |
| Appended foreign stream from a second top-level `ftyp` (Samsung motion-photo MP4) retained opaque | `references/heif` §8b | ✅ | S1 |
| Trailing non-box bytes (Samsung SEF trailer) retained as `SegmentKind::Trailer` (post ftyp+meta) | `references/heif` §8b | ✅ | S1 |
| Stop rules identical to `gamut_isobmff::read` (first ftyp wins; trailer only after ftyp+meta) | 14496-12 | ✅ | S1 |
| Meta-level accounting: `meta`/`iprp` children not consumed by the model surfaced as `UnknownBox` (e.g. `dinf`/`dref`, `uuid`) | 14496-12 | ✅ | S1 |
| `ftyp` brands + `is_hevc_still` (`heic`/`heix`/`heim`/`heis`, or `mif1`+`hvcC` primary) | 23008-12; `references/heif` §7 | ✅ | S1 |
| Sequence brands `msf1`/`hevc`/`hevx` (image sequences) | `references/heif` §7 | OOS | OOS |

## B. Item model / roles (23008-12 · MIAF)

| Component | Spec | Status | Slice |
| --- | --- | --- | --- |
| Primary-item validation: `pitm` names an existing, non-hidden item | 23008-12 | ✅ | S1 |
| Item kind from `item_type`: coded (`hvc1`/`hev1`/`av01`…), `grid`/`iovl`/`iden`, `Exif`/`mime` | 23008-12 §6 | ✅ | S1 |
| `hvc1` vs `hev1` inband-parameter-set distinction | 14496-15 §8.3.2; `references/heif` §2 | ✅ | S1 |
| Descriptive properties: `ispe`/`pixi`/`colr`/`clli` typed accessors | 23008-12 §6.5 | ✅ | S1 |
| Transformative properties `clap`/`irot`/`imir` in `ipma` order + MIAF order check | 23008-12 §7; MIAF; `references/heif` §5 | ✅ | S1 |
| `pasp` pixel aspect ratio accessor | 14496-12 §12.1.4 | ✅ | S1 |
| Unsupported-essential-property flag (MIAF: don't render an unknown essential property) | MIAF §7.3.6 | ✅ | S1 |
| Raw `hvcC`/`av1C` codec configuration exposed as opaque `(type, body)` | 14496-15 §8.3.3 | ✅ | S1 |
| Thumbnail lens (`thmb`) / auxiliary lens (`auxl`) | 23008-12 §6 | ✅ | S1 |
| Alpha/depth auxiliary by `auxC` URN (`hevc:2015:auxid:1/2`, `mpegB:…:alpha/depth`) | 23008-12 §6.5.8; MIAF §7.3.5; `references/heif` §6 | ✅ | S1 |
| Premultiplied-alpha (`prem`) relationship | 23008-12 §6; `references/heif` §6 | ✅ | S1 |
| Metadata lens: Exif/XMP items via `cdsc` (metadata → image) + `exif()`/`xmp()` | 23008-12 §A; `references/heif` §9 | ✅ | S1 |
| Derived-image sources (`dimg`), `grid` payload + tile-count validation, `iovl` payload | 23008-12 §6.6.2; `references/heif` §4 | ✅ | S1 |
| `iden` identity derived item recognised (kind); source via `dimg` | 23008-12 §6.6.2.1 | ✅ | S1 |
| Entity groups + `altr` alternatives lens | 14496-12; MIAF | ✅ | S1 |
| Decoded Exif/XMP bytes → `gamut-exif`/`gamut-xmp` (payload exposed opaque here) | 23008-12 §A | ☐ | S3 |
| Protected / `uri ` items; external data references | 23008-12 | OOS | OOS |

## C. HEVC configuration & NAL layer (14496-15 · H.265)

| Component | Spec | Status | Slice |
| --- | --- | --- | --- |
| Typed `hvcC` HEVCDecoderConfigurationRecord parse (profile/tier/level, arrays) — `HevcConfig::parse` (`src/hvcc.rs`), reached via `HeifItem::hevc_config` | 14496-15 §8.3.3.1; `references/heif` §1 | ✅ | S2 |
| Item payload → length-prefixed NAL unit split (`lengthSizeMinusOne`) — `iter_nal_units`/`NalUnitIter` (`src/nal.rs`) | 14496-15 §8.3.2; `references/heif` §2 | ✅ | S2 |
| NAL unit header classify (VPS/SPS/PPS/SEI/IRAP) + still-image IRAP constraint — `NalUnitType`/`NalHeader` (`src/nal.rs`), `HevcConfig::validate_still_payload` | H.265 §7.3.1.2; `references/heif` §3 | ✅ | S2 |
| Annex-B conversion for a downstream decoder — `HevcConfig::annex_b` (`src/hvcc.rs`) | 14496-15 §8.3.2 | ✅ | S2 |
| L-HEVC multi-layer decode (`heim`/`heis` beyond base layer) | 14496-15 | OOS | OOS |

## D. Pixel decode & API (H.265 intra · gamut-core) — deferred

| Component | Spec | Status | Slice |
| --- | --- | --- | --- |
| `gamut_core::DecodeImage` impl (pluggable codestream-decoder hook) | gamut-core; #238 | ☐ | S3 |
| HEVC-intra reconstruction (slice/CTU/transform/intra-pred/in-loop filters) | H.265 (ITU-T) | ☐ | S3 |
| `grid`/`iovl`/`iden` derived-image compositing to pixels | 23008-12 §6.6.2 | ☐ | S3 |
| Transformative-property application (`clap`→`irot`→`imir`) to output pixels | 23008-12 §7; MIAF | ☐ | S3 |
| HEVC inter coding (motion, reference frames, sequences) | H.265 | OOS | OOS |
| CLI / wasm / ffi wiring | gamut-{cli,wasm,ffi} | ☐ | S3 |

## E. Conformance oracle — deferred

| Component | Spec | Status | Slice |
| --- | --- | --- | --- |
| libheif (libde265) differential parse + decode oracle (`tooling/…-oracle` over a `third_party/` submodule) | `references/heif` "Oracle" | ☐ | S4 |
| Nokia HEIF reference software as a secondary fixture source | `references/heif` "Oracle" | ☐ | S4 |

## The S1 guarantee

`gamut-heic`'s container slice promises: `HeifContainer::parse` accounts for **every byte** of the
input (the segments tile `0..len` exactly, by construction and pinned by tests), surfaces every
unknown top-level and `meta`/`iprp` box verbatim, retains appended vendor streams and trailers as
opaque byte ranges, and exposes a role-typed view whose accessors are computed lenses over the
single-source-of-truth `gamut_isobmff::IsoBmffImage` — no state duplicated, the primary item
validated at parse. Every deferred row lands additively; the S1 public surface is never reshaped.
