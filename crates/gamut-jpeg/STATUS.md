# gamut-jpeg — JPEG-1 codec status

Tracking GitHub issue #28: a spec-compliant JPEG-1 (ISO/IEC 10918-1 | ITU-T T.81) still-image
**encoder + decoder**. Delivered as small, individually green phases (each `mise run test`/`lint`/
`fmt-check`/`coverage` ≥80%).

**Keystone:** the SOI → APP0(JFIF) → DQT → SOF0 → DHT → SOS → entropy → EOI pipeline with the
baseline sequential DCT Huffman encoder (P1) — once libjpeg-turbo decodes that to the source pixels
(within the lossy tolerance), each later phase adds a process (progressive) or a direction (decode)
behind the same marker spine, entropy coder, and table machinery.

**Oracle:** differential vs **libjpeg-turbo** (a vendored, dev-only static build, landing in P3),
cross-checked against the vendored **T.873 reference software** (`references/jpeg`, ISO/IEC 10918-7)
for spec-exact behaviour. gamut ships no JPEG decoder yet, so until P2 the encoder is pinned by
byte-exact micro-goldens hand-derived from T.81 Annex F/K and a structural stream walker; P3 adds
the round-trip gate (libjpeg-turbo decodes the encoder's output → matches source within tolerance,
and gamut's decoder decodes libjpeg-turbo's output).

## Scope ledger

**In scope** (issue #28, across the phases below):

- Baseline sequential DCT, Huffman, 8-bit (SOF0) **encode** — grayscale + YCbCr 4:4:4/4:2:2/4:2:0,
  JFIF interchange format (this phase).
- Sequential DCT, Huffman, 8-bit (SOF0 baseline / SOF1 extended) **decode**.
- Progressive DCT, Huffman (SOF2) **decode and encode** (spectral selection + successive
  approximation).
- Restart markers (DRI/RSTn).
- Colour-space handling: JFIF (APP0) and Adobe (APP14) transform flags; CMYK / YCCK **decode**.

**Deferred / out of scope** (documented, with reasons):

- **12-bit precision (P=12).** Baseline is 8-bit only; 12-bit sample precision is an extended-DCT
  feature with little real-world corpus. The DCT kernel (`gamut-dsp`) already handles it, so this is
  additive later if demand appears.
- **Arithmetic coding (SOF9/SOF10, DAC).** Patent-era, near-absent in the wild; Huffman covers all
  practical JPEG-1. Not planned.
- **Lossless process (SOF3).** The predictive lossless mode is unrelated to the DCT pipeline;
  `gamut-dng` covers the only in-workspace consumer (lossless-JPEG inside DNG). Not planned here.
- **Hierarchical mode (SOF5–7, DHP, EXP).** No real-world still-image corpus. Not planned.
- **SPIFF and other T.84 extensions; T.872 printing conventions.** Niche container/printing layers
  atop the codec, tracked in `references/jpeg` but not implemented.

**Decoder-specific notes:**

- **DNL (define number of lines, §B.2.5).** The **decoder** parses DNL and resolves a frame with
  `Y = 0` by decoding MCU rows until the entropy data ends at a marker (the `Y = 0` frame's height
  then arrives in the following DNL). A DNL after a `Y ≠ 0` frame is advisory and ignored. The
  encoder never emits `Y = 0` (it always knows its height and writes it in SOF0).
- **Upsampling filter.** Subsampled chroma is upsampled by **sample replication** (nearest); T.81
  leaves the reconstruction filter open (§A.2 NOTE), so this is the decoder's documented free choice.
- **Trailing bytes after EOI** are ignored (the libjpeg convention).
- **CMYK is presented verbatim** (no Adobe inversion); **YCCK** applies the inverse YCbCr transform to
  the first three channels with `K` passed through (Adobe TN #5116).

## Phases

| Phase | Spec | Scope | Status |
| ----- | ---- | ----- | ------ |
| P1 | T.81 Annex A/B/C/F/K; T.871 | **Keystone:** scaffold + workspace wiring; marker/syntax layer; baseline SOF0 Huffman **encoder** (gray + YCbCr 4:4:4/4:2:2/4:2:0), Annex K tables, quality scaling, restart intervals, JFIF | ✅ done |
| P2 | T.81 Annex A/B/C/F; T.871; TN #5116 | Sequential SOF0/SOF1 8-bit Huffman **decoder**: full marker/table parsing (DQT 8/16-bit, DHT with Annex C validation, DRI, DNL, APP0/APP14), interleaved + non-interleaved multi-scan entropy decode (§F.2), restart processing, gray/YCbCr/RGB/CMYK/YCCK colour, sample-replication upsampling, `info()` | ✅ done |
| P3 | T.83 / oracle | libjpeg-turbo differential oracle (vendored, dev-only) + round-trip gate | ⏳ pending |
| P4 | T.81 §G | Progressive SOF2 **decode** (spectral selection + successive approximation) | ⏳ pending |
| P5 | T.81 §G | Progressive SOF2 **encode** | ⏳ pending |
| P6 | — | Hardening: CMYK/YCCK + Adobe APP14, CLI `gamut convert → .jpg`, umbrella `jpeg` feature audit | ⏳ pending |
