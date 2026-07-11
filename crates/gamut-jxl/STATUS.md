# gamut-jxl — JPEG XL implementation status

Tracking GitHub issue [#243](https://github.com/justin13888/gamut/issues/243): integrate a JPEG XL
**encoder and decoder** into gamut. Unlike every other codec here, gamut-jxl **wraps the format's
reference implementations** rather than implementing the ISO/IEC 18181 bitstream clean-slate —
libjxl for encode, jxl-rs for decode — a deliberate, maintainer-confirmed departure justified in the
crate [README](README.md#why-a-wrapper).

**Architecture:** encode wraps **libjxl v0.12.0** (statically linked via
[`gamut-jxl-sys`](../gamut-jxl-sys); source vendored by the BSD-3-Clause `jpegxl-src = "=0.12.0"`,
bundled skcms, no lcms2); decode wraps the pure-Rust [`jxl` crate v0.4.3](https://crates.io/crates/jxl)
(jxl-rs). The encoder is target-gated off `wasm32`, where the crate becomes decode-only.

**Oracle:** libjxl v0.12.0 is both the encode core and the decode **oracle** — the same static
archive exposes a decode surface (via `gamut-jxl-sys`) that the differential tests cross-check the
pure-Rust jxl-rs decoder against (see [Oracle & test regime](#oracle--test-regime)). Because the
encoder itself *is* the reference codec, correctness is anchored on decoders **agreeing** rather than
on a hand-written golden bitstream.

## Implemented (v1 surface)

- **Lossless (Modular).** `JxlEncoder::lossless()` (also `new`/`Default`) → libjxl Modular via
  `SetFrameLossless` + `uses_original_profile`; the decoded image is bit-exact to the input.
- **Lossy (VarDCT / XYB).** `JxlEncoder::lossy(Distance)` → VarDCT in XYB by Butteraugli
  `SetFrameDistance`. `Distance` is a validated newtype over the half-open `(0.0, 25.0]`; `1.0` is
  "visually lossless" and the default. `0.0` (libjxl's lossless sentinel) is rejected so the two
  modes stay structurally distinct.
- **Effort dial.** `Effort` `Lightning..=Glacier`, mapping libjxl effort `1..=10` (default
  `Squirrel` = 7). Level 11 ("tectonic plate") is expert-gated and out of scope.
- **Pixel layouts.** 8- and 16-bit **Gray / GrayAlpha / RGB / RGBA** (eight `EncodeImage` /
  `DecodeImage` impls); 16-bit samples handed to libjxl as native-endian bytes.
- **Container framing.** Bare codestream (default, signature `FF 0A`) and the ISO BMFF `.jxl`
  container (`Container`).
- **Colour signalling.** sRGB (`JxlColorEncodingSetToSRGB`, gray or colour).
- **Full pixel decode (jxl-rs).** Decodes the entire ISO/IEC 18181-1 pixel surface jxl-rs
  covers — VarDCT and Modular (RCT/palette/squeeze), XYB, splines/patches/noise/spot colours,
  progressive-encoded streams, and both `jxlc`/`jxlp` container framings — reshaping to the
  requested layout losslessly (grayscale→RGB, opaque-alpha pad, alpha drop).
- **Decode policies.** Pixel-limit bound (`1 << 28` samples); truncated → `InvalidInput`; animation,
  premultiplied (associated) alpha, and colour-as-grayscale each → `Unsupported` (deliberate refusals
  to guess, additive to relax later).

## Deferred (planned; additive — semver-minor, no surface reshape)

Each is a self-contained follow-up that plugs into the existing wrapper; the one-line note says what
unlocks it.

- **JPEG recompression (jbrd).** `JxlEncoder::recompress_jpeg` is the reserved API slot; it returns
  `Unsupported` today. Unlocks with libjxl's `AddJPEGFrame` + the container `jbrd` box (encode side is
  available in libjxl 0.12.0; jxl-rs reconstruction is still in-flight — see the ledger below).
- **Custom colour (ICC / CICP) and HDR.** Only sRGB is signalled today. Needs ICC-blob / CICP plumbing
  through `SetColorEncoding` (encode) and a colour-management transform on decode (PQ/HLG transfer,
  intensity target); jxl-rs currently returns its ICC/CMS paths as `Unsupported` here.
- **Premultiplied (associated) alpha decode.** Rejected today; unlocks with an un-premultiply step in
  `convert`.
- **Progressive encode control.** No passes / group-order / responsive knobs are exposed; unlocks with
  the corresponding `FrameSettingsSetOption` calls plus a config surface.
- **Extra channels beyond alpha.** Depth, thermal, spot, and other extra channels are ignored on
  decode and unsupported on encode; unlocks with a typed extra-channel model.
- **Effort 11 ("tectonic plate").** Expert-gated behind `JxlEncoderAllowExpertOptions`; the `Effort`
  enum caps at 10 by design.
- **Exif / XMP container metadata.** No metadata boxes are written or surfaced; unlocks with the future
  `gamut-metadata` integration (issue #34) writing `Exif`/`xml ` boxes into the ISO BMFF container.
- **Streaming / partial decode API.** The decoder consumes a whole buffer per call; a chunked/streaming
  entry point is a separate additive surface.
- **libjxl 0.12.x tracking.** The pin is exact (`jpegxl-src = "=0.12.0"` → libjxl 0.12.0). Bumps are
  deliberate: they must re-verify the FFI declarations against the new headers via the
  `gamut-jxl-sys` version-pin / symbol-drift test (`tests/version.rs`, asserting version `12000`) and
  re-run the differential suite.

## Out of scope (image-first charter; rejected with a typed error)

Relaxing any of these is additive if the charter ever changes, but none is planned.

- **Animation — encode *and* decode.** gamut implements no multi-frame/sequence coding; an animated
  input stream decodes to `Unsupported("JXL: animated JPEG XL is not supported")`, and no animated
  stream is emitted.
- **Preview frames.** The small embedded preview image is neither produced nor surfaced.
- **CMYK.** Requires an ICC path (JXL has no native CMYK signalling); parsed-but-not-presentable in
  jxl-rs, and out of scope for the encoder.

## jxl-rs spec-coverage ledger (issue #243 deliverable)

jxl-rs 0.4.3 covers essentially the full ISO/IEC 18181-1 **pixel-decode** surface (VarDCT, Modular
incl. RCT/palette/squeeze, XYB, splines/patches/noise, spot colours, ICC incl.
compressed-restricted, CICP, the `jxlc`/`jxlp`/`jxli` container framings, animation, progressive).
The documented gaps, with upstream links (verified against the tracker 2026-07-11):

- **No encoder.** jxl-rs ships a decoder only — the reason gamut wraps libjxl for the encode half.
- **jbrd JPEG reconstruction.** Bit-exact JPEG-from-jbrd reconstruction is **not yet in a release**:
  it lands with [libjxl/jxl-rs#590](https://github.com/libjxl/jxl-rs/pull/590) (open PR, behind a
  `jpeg-reconstruction` flag, disabled by default). Separately,
  [#513](https://github.com/libjxl/jxl-rs/issues/513) requests using the JXL decoder to decode
  *legacy* JPEG files (a distinct feature; open). gamut's `recompress_jpeg` slot stays deferred
  regardless.
- **HDR tone-mapping render stage.** Tracked as a WIP item on the render-stage tracking bug
  [#58](https://github.com/libjxl/jxl-rs/issues/58) — one reason custom-colour/HDR decode is deferred.
- **Progressive / preview corners.** Unimplemented progressive types incl. preview frames and
  LfFrame-with-alpha ([#730](https://github.com/libjxl/jxl-rs/issues/730)); `flush_pixels` bugs
  ([#783](https://github.com/libjxl/jxl-rs/issues/783),
  [#771](https://github.com/libjxl/jxl-rs/issues/771)); high memory on progressive lossless
  ([#782](https://github.com/libjxl/jxl-rs/issues/782)). gamut decodes complete (non-streaming)
  streams, so it does not exercise these flush/partial paths.
- **Container Exif / XMP metadata not exposed.** jxl-rs does not surface the `Exif`/`xml ` box bytes
  ([#674](https://github.com/libjxl/jxl-rs/issues/674)) — the upstream half of gamut's deferred
  metadata work.
- **CMYK.** Parsed but not presentable (see Out of scope).

## Oracle & test regime

- **Oracle:** libjxl v0.12.0, reached through the `gamut-jxl-sys` decode bindings (a `tests/common`
  unsafe helper). The `gamut-jxl-sys` `tests/version.rs` pins `JxlEncoderVersion()` /
  `JxlDecoderVersion()` to `12000` so a header/transcription drift is caught.
- **Lossless — three-way bit-exact.** For all eight layouts across a size grid (incl. odd sizes) and
  both container framings: encode with libjxl, decode with jxl-rs, and the result is **bit-exact to
  the source**; the libjxl oracle decode of the same stream agrees, giving a source ⇄ gamut ⇄ libjxl
  three-way match.
- **Lossy — bounded agreement + PSNR floors.** At distances `{0.5, 1.0, 3.0}` on smooth generators,
  the jxl-rs and libjxl decoders agree within **≤ 2** per 8-bit sample / **≤ 514** per 16-bit sample,
  each stays above a **PSNR floor** (≥ 35 dB at distance 1.0), lossy bytes differ from lossless, and a
  larger distance yields a smaller file. (PSNR is content-dependent: distance-1.0 is Butteraugli-, not
  PSNR-, targeted, so the floors use smooth content by design.)
- **Feature grid + robustness.** A differential feature matrix (container × naked, effort extremes,
  16-bit, alpha, grayscale, gray+alpha) plus a large hostile-input corpus (empty,
  short prefixes, systematic truncations, garbage bodies, and bit-flips across the first 256 bytes,
  and the pixel-limit trigger): every case returns a typed `Err`, never a panic.
- **Orientation — not yet exercised.** The gamut encoder emits identity orientation only, and decode
  follows jxl-rs's display-oriented `basic_info.size`; non-identity orientations (rotated/mirrored
  streams from foreign encoders) are not yet covered by the test suite.
- **Signatures / conversions.** Codestream vs. container signature bytes and the
  gray→RGB / RGBA→RGB / RGB→gray-rejection conversion contracts are pinned.
- **Mutants:** zero unjustified survivors; the only `exclude_re` entries carry strong justifications
  (encoder free choices whose output is unobservable through the decoders).
