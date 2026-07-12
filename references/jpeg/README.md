# JPEG (JPEG-1, ISO/IEC 10918 / ITU-T T.8x)

Reference specifications for a future **`gamut-jpeg`** codec — the baseline/extended DCT-based
still-image process (JPEG-1) plus its file formats (JFIF) and marker/registration companions. These
also document the JPEG constructs consumed elsewhere in the workspace: JPEG-in-TIFF (`gamut-tiff`
compression `6`/`7`, TIFF 6.0 §22 + Technical Note 2) and lossless-JPEG in DNG (`gamut-dng`).

JPEG-1 is published jointly as the **ISO/IEC 10918** multi-part standard and the **ITU-T T.8x**
Recommendations; the two are technically identical, and ITU freely publishes the T-series text while
the ISO/IEC parts are paywalled. The files below are the freely-published ITU editions.

## The ISO/IEC 10918 ↔ ITU-T map

| ISO/IEC | ITU-T | Title | Vendored |
| --- | --- | --- | --- |
| 10918-1 | T.81 | Requirements and guidelines (the core codec) | ✅ `itu-t81.pdf` (+ Cor.1) |
| 10918-2 | T.83 | Compliance testing | ✗ (see *Not vendored*) |
| 10918-3 | T.84 | Extensions | ✅ `T-REC-T.84-…` (+ Amd.1) |
| 10918-4 | T.86 | Registration of profiles/APPn markers (REGAUT) | ✅ `T-REC-T.86-…` |
| 10918-5 | T.871 | JPEG File Interchange Format (JFIF) | ✅ `T-REC-T.871-…` (+ Err.1) |
| 10918-6 | T.872 | Application to printing systems | ✅ `T-REC-T.872-…` |
| 10918-7 | T.873 | Reference software | ✅ `T-REC-T.873-…SOFT-ZST-E.zip` |

One companion outside the 10918 family is also vendored because a spec-complete codec needs it: **Adobe
Technical Note #5116** (`APP14` colour-transform marker) — see *Adobe `APP14` colour transform* below.

## Authoritative editions (vendored)

### Core codec — 10918-1 / T.81

- `itu-t81.pdf` — **ITU-T Rec. T.81 (09/1992) | ISO/IEC 10918-1** — *Information technology — Digital
  compression and coding of continuous-tone still images — Requirements and guidelines.* The
  foundational JPEG-1 specification: the baseline and extended sequential/progressive DCT processes,
  the lossless process, entropy coding (Huffman and arithmetic), the marker syntax (SOI/SOF/SOS/DQT/
  DHT/DRI/…), and the interchange/abbreviated formats. This is the freely-redistributed copy hosted by
  the W3C (`https://www.w3.org/Graphics/JPEG/itu-t81.pdf`); it is byte-for-byte the 1992 text.
- `T-REC-T.81-200401-I!Cor1!PDF-E.pdf` — **T.81 (1992) Technical Corrigendum 1 (01/2004)** — patent
  information update only; no normative change to the codec. Retained for completeness.

### Extensions — 10918-3 / T.84

- `T-REC-T.84-199607-I!!PDF-E.pdf` — **ITU-T Rec. T.84 (07/1996) | ISO/IEC 10918-3** — *…continuous-tone
  still images: Extensions.* Adds the variable quantization, selective refinement, tiling, still-picture
  interchange (SPIFF) file format, and the extended registration mechanisms layered on T.81.
- `T-REC-T.84-199904-I!Amd1!PDF-E.pdf` — **T.84 Amendment 1 (04/1999)** — *Provisions to allow
  registration of new compression types and versions in the SPIFF header.*

### Marker & file-format companions

- `T-REC-T.86-202402-I!!PDF-E.pdf` — **ITU-T Rec. T.86 (V2) (02/2024) | ISO/IEC 10918-4** —
  *…continuous-tone still images: APPn markers* (the Registration Authority / REGAUT document). Defines
  the registration of JPEG profiles, SPIFF profiles/tags/colour spaces, `APPn` markers, and compression
  types. This is the current (2024) edition.
- `T-REC-T.871-201105-I!!PDF-E.pdf` — **ITU-T Rec. T.871 (05/2011) | ISO/IEC 10918-5** — *…continuous-tone
  still images: JPEG File Interchange Format (JFIF).* The de-facto container for JPEG-1 in the wild: the
  `APP0`/`JFIF` and `JFXX` marker segments, aspect ratio/density, and thumbnail conventions.
- `T-REC-T.871-201303-I!Err1!PDF-E.pdf` — **T.871 Erratum 1 (03/2013)** — corrects the publication date
  in the page headers to reflect ISO/IEC 10918-5:2013; editorial only.
- `T-REC-T.872-201206-I!!PDF-E.pdf` — **ITU-T Rec. T.872 (06/2012) | ISO/IEC 10918-6** — *…continuous-tone
  still images: Application to printing systems.* Defines the `APP11`/print-industry conventions.

### Adobe `APP14` colour transform — Technical Note #5116 (non-ISO, but required)

- `adobe-tn5116-dct-filter.pdf` — **Adobe Technical Note #5116, *Supporting the DCT Filters in PostScript
  Level 2* (24 November 1992).** *Not* part of ISO/IEC 10918 / ITU-T T.8x, and the only vendored companion
  outside that family — but required for correct real-world JPEG-1. It defines the `APP14`/`Adobe` marker
  segment whose colour-transform flag (`0` = unknown → RGB or CMYK, `1` = YCbCr, `2` = YCCK) is the
  authoritative signal for whether a decoder applies the inverse colour transform on 3-component non-JFIF
  images and on all 4-component (CMYK/YCCK) images; JFIF/SPIFF/T.81 provide no equivalent. The intended
  libjpeg-turbo/mozjpeg oracle emits and consumes this marker, so encode/decode parity requires handling
  it. Freely published by the PDF Association as an ISO 32000 normative reference; downloaded from
  `https://pdfa.org/norm-refs/5116.DCT_Filter.pdf`.

### Reference software — 10918-7 / T.873

- `T-REC-T.873-202309-I!!SOFT-ZST-E.zip` — **ITU-T Rec. T.873 (09/2023) | ISO/IEC 10918-7 — Reference
  software (V3).** The official JPEG-1 reference implementation. The archive bundles the T.873 PDF plus
  two reference codebases (`reference A`, `reference B`); it is the normative behavioural oracle for the
  core codec.

## Not vendored (paywalled / cross-checked via oracle)

- **ISO/IEC 10918-2 (ITU-T T.83) — Compliance testing** — the conformance test-data part. Not freely
  published by ITU (no T-series companion is posted) and paywalled at ISO; per the same policy applied to
  the paywalled ISOBMFF/TIFF-EP specs, JPEG-1 conformance is instead exercised against the vendored T.873
  reference software and a C reference codec (see *Conformance*), rather than vendoring this part.

## Conformance

The intended correctness strategy mirrors the other codec crates: a **differential oracle** against a
canonical C JPEG-1 implementation (libjpeg-turbo / mozjpeg family) plus the vendored **T.873 reference
software** for spec-exact behaviour, gating decode round-trips and encode parity. The concrete oracle
crate and gate will be documented in `gamut-jpeg`'s `STATUS.md` when the crate lands.
