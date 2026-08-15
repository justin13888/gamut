# gamut-jxl — JPEG XL implementation status

Tracking GitHub issue [#243](https://github.com/justin13888/gamut/issues/243): integrate a JPEG XL
**encoder and decoder** into gamut. Unlike every other codec here, gamut-jxl **wraps the format's
reference implementations** rather than implementing the ISO/IEC 18181 bitstream clean-slate —
libjxl for encode, jxl-rs for decode — a deliberate, maintainer-confirmed departure justified in the
crate [README](README.md#why-a-wrapper).

**Architecture:** encode wraps **libjxl v0.12.0** (statically linked via
[`gamut-jxl-sys`](../gamut-jxl-sys); source vendored by the BSD-3-Clause `jpegxl-src = "=0.12.0"`,
bundled skcms, no lcms2); decode wraps the pure-Rust [`jxl` crate v0.4.3](https://crates.io/crates/jxl)
(jxl-rs). The encoder is available on all targets except `wasm32` without emscripten:
`wasm32-unknown-emscripten` links the emsdk-built libjxl (full encoder), while
`wasm32-unknown-unknown`/`wasm32-wasip*` are decode-only — no C/C++ toolchain targets those ABIs.

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
- **JPEG recompression (jbrd).** `JxlEncoder::recompress_jpeg` losslessly transcodes a JPEG
  codestream via `AddJPEGFrame` + `StoreJPEGMetadata`; the original `.jpg` is reconstructible
  **bit-for-bit** (proven against the libjxl oracle) and the stream still decodes as ordinary
  pixels. Output is always container-framed (the `jbrd` box requires it); only `Effort` applies.
- **Effort dial.** `Effort` `Lightning..=Glacier`, mapping libjxl effort `1..=10` (default
  `Squirrel` = 7). Level 11 ("tectonic plate") is expert-gated and out of scope.
- **Modular-mode control (issue #339).** `ModularMode` `Auto`/`VarDct`/`Modular` via `with_modular`,
  mapping libjxl's `JXL_ENC_FRAME_SETTING_MODULAR` values `-1`/`0`/`1`. `Auto` is the default and
  leaves the option **unsent**, so a default encoder's bytes are unchanged. Forcing `VarDct` on a
  lossless encoder is a typed `InvalidInput` rather than a silent no-op: libjxl's `SetLossless`
  overrides the frame setting to modular unconditionally, so the request could not be honoured.
  `ModularMode` is a codestream-level knob, so it reaches pushed backends through `JxlEncodeRequest`;
  the `gamut-codec-abi` adapter has no `EncodeConfig` field for it and therefore **declines** a
  pinned mode (as it already does for colour and orientation) instead of dropping it. It does not
  apply to `recompress_jpeg`, which re-packs the JPEG's own coefficients — a non-default setting is
  ignored there, as the other inapplicable knobs already are.
- **Pixel layouts.** 8- and 16-bit **Gray / GrayAlpha / RGB / RGBA** (eight `EncodeImage` /
  `DecodeImage` impls); 16-bit samples handed to libjxl as native-endian bytes.
- **Container framing.** Bare codestream (default, signature `FF 0A`) and the ISO BMFF `.jxl`
  container (`Container`).
- **Colour signalling (`ColorSpec`).** sRGB (default), linear sRGB, **PQ** and **HLG** (BT.2100/D65
  structured encodings, HDR-coded `u16` samples), and a verbatim embedded **ICC profile**
  (`SetICCProfile`, with a structural gray/RGB pre-check). Signalling only — the encoder never
  converts pixels. The decoder surfaces embedded ICC bytes via `JxlDecoder::embedded_icc_profile`.
- **Orientation.** `Orientation` (all eight EXIF values) via `with_orientation`; metadata-only
  (samples stay in coded order, decoders apply the transform).
- **Exif / XMP container boxes.** `with_exif` (raw EXIF; the 4-byte tiff-offset prefix is added
  automatically) and `with_xmp`, written as uncompressed `Exif` / `xml ` boxes; requires
  `Container::IsoBmff` (a typed error otherwise).
- **Full pixel decode (jxl-rs).** Decodes the entire ISO/IEC 18181-1 pixel surface jxl-rs
  covers — VarDCT and Modular (RCT/palette/squeeze), XYB, splines/patches/noise/spot colours,
  progressive-encoded streams, and both `jxlc`/`jxlp` container framings — reshaping to the
  requested layout through `gamut_core::convert` (issue #268). `crate::convert` now keeps only the
  jxl-specific half — reassembling jxl-rs's native-endian output *bytes* into typed samples — at the
  cost of one extra pass and allocation the previously fused loop avoided.
- **Decode policies.** Pixel-limit bound (`1 << 28` samples); truncated → `InvalidInput`; animation
  and premultiplied (associated) alpha → `Unsupported`. Layout loss (dropping a present alpha
  channel, reducing colour to grayscale) is refused by default and enabled per decoder with
  `JxlDecoder::with_convert_policy` — the refusal is no longer unconditional.
- **Pluggable codestream backends (issue #276).** Both directions are registries over the shared
  `gamut-codec-abi` seam, cut at the **bare `FF 0A` codestream**: `JxlEncoder::push_backend` /
  `JxlDecoder::push_backend` insert a `JxlCodestreamEncoder` / `JxlCodestreamDecoder` ahead of the
  built-in wrappers, which are the implicit **tails**. Push order; `supports() == false` (or a late
  `Error::Unsupported`, the typed mirror of `Status::UNSUPPORTED`) is the only fall-through, and an
  accepted-then-failed backend propagates. `AbiEncodeBackend` / `AbiDecodeBackend` adapt a
  `gamut_codec_abi::Encoder` / `Decoder` (including a C vtable via `bridge::Foreign*`) onto the same
  door under `JXL_CODEC_ID` (`"jxl "`). Container-dependent features — ISO BMFF output,
  `with_exif`/`with_xmp`, and `recompress_jpeg` — are pinned to the built-in path by a **host-side
  veto**: the registry is not consulted at all. Consequently the `encode`/`decode` features now mean
  "**include the built-in tail**", not "enable the direction".
- **WebAssembly.** Decode on every `wasm32` target; **encode on `wasm32-unknown-emscripten`**
  (emsdk-built libjxl, full differential suite in the extended-CI wasm lane).
  `wasm32-unknown-unknown` stays decode-only — a toolchain boundary, not a workaround.

## Deferred (planned; additive — semver-minor, no surface reshape)

Each is a self-contained follow-up that plugs into the existing wrapper; the one-line note says what
unlocks it.

- **Decode-side colour management (CMS transforms).** The decoder returns samples in the stream's
  own colour encoding and surfaces embedded ICC bytes, but applies no ICC transform and no HDR→SDR
  tone mapping (PQ/HLG intensity handling); unlocks with a CMS/tone-mapping stage (see
  `gamut-tonemap`) once jxl-rs's HDR render stages settle (ledger below).
- **JPEG reconstruction on decode.** gamut writes `jbrd` streams whose original JPEG the *libjxl*
  decoder reconstructs bit-for-bit; a pure-Rust reconstruction API is blocked on jxl-rs shipping
  its `jpeg-reconstruction` feature (ledger below).
- **Reading Exif / XMP boxes back on decode.** gamut writes the boxes; surfacing them from incoming
  streams is blocked on jxl-rs exposing box contents (ledger below) and ties into the
  `gamut-metadata` facade (issue #34) for typed parsing.
- **Premultiplied (associated) alpha decode.** Rejected today; unlocks with an un-premultiply step in
  `convert` (deliberately deferred: an integer un-premultiply is an approximate inverse — alpha = 0
  is unrecoverable — so it belongs behind an explicit opt-in, not a silent default).
- **Frame settings beyond effort and modular mode.** Progressive control (passes / group order /
  responsive), the modular *tuning* knobs (`MODULAR_COLOR_SPACE`, `MODULAR_GROUP_SIZE`,
  `MODULAR_PREDICTOR`, `MODULAR_NB_PREV_CHANNELS`, and the float-valued
  `MODULAR_MA_TREE_LEARNING_PERCENT`, which additionally needs
  `JxlEncoderFrameSettingsSetFloatOption` declared) and the coding-tool toggles (noise, dots,
  patches, EPF, gaborish) are not exposed. Each unlocks with its `gamut-jxl-sys` enumerant plus a
  config surface; issue #339 opened that seam, so these are now additive rather than an FFI change
  each time.
- **Extra channels beyond alpha.** Depth, thermal, spot, and other extra channels are ignored on
  decode and unsupported on encode; unlocks with a typed extra-channel model.
- **Effort 11 ("tectonic plate").** Expert-gated behind `JxlEncoderAllowExpertOptions`; the `Effort`
  enum caps at 10 by design (also: adding a variant to the exhaustive public enum is semver-major).
- **Container ownership (follow-up to issue #276).** ISO BMFF box writing and `jbrd` JPEG
  reconstruction metadata are produced *by libjxl* today, which is why the container features are
  vetoed away from pushed backends. Moving that box/`jbrd` writing into gamut-jxl proper — building
  the `.jxl` container over *any* backend's codestream, most naturally on `gamut-isobmff` — would let
  a pushed backend serve container output and metadata embedding too. Recorded, **not implemented**;
  it is purely additive to the seam (the traits and the fallback contract do not change).
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
  *legacy* JPEG files (a distinct feature; open). gamut's `recompress_jpeg` **encode** side ships
  (libjxl-backed, oracle-verified bit-exact reconstruction); a pure-Rust *decode-side*
  reconstruction API stays blocked on the upstream release. jxl-rs decodes the *pixels* of jbrd
  streams today (covered by tests).
- **HDR tone-mapping render stage.** Tracked as a WIP item on the render-stage tracking bug
  [#58](https://github.com/libjxl/jxl-rs/issues/58). gamut's tests confirm jxl-rs renders lossy
  XYB back to the embedded PQ encoding, but HDR→SDR tone mapping and general CMS transforms remain
  the reason decode-side colour *management* is deferred.
- **Progressive / preview corners.** Unimplemented progressive types incl. preview frames and
  LfFrame-with-alpha ([#730](https://github.com/libjxl/jxl-rs/issues/730)); `flush_pixels` bugs
  ([#783](https://github.com/libjxl/jxl-rs/issues/783),
  [#771](https://github.com/libjxl/jxl-rs/issues/771)); high memory on progressive lossless
  ([#782](https://github.com/libjxl/jxl-rs/issues/782)). gamut decodes complete (non-streaming)
  streams, so it does not exercise these flush/partial paths.
- **Container Exif / XMP metadata not exposed.** jxl-rs does not surface the `Exif`/`xml ` box bytes
  ([#674](https://github.com/libjxl/jxl-rs/issues/674)) — the upstream half of gamut's deferred
  *read-back* support (gamut's encoder writes the boxes today).
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
- **Coding tool — plumbed, and `Auto` provably inert.** Neither decoder reports whether a stream is
  VarDCT or Modular, so `ModularMode` is pinned the way `Effort` is: forcing Modular must change the
  stream bytes (against both forced VarDCT and libjxl's own choice) while staying decodable by
  jxl-rs *and* the oracle, and a forced-Modular lossy stream must still clear the PSNR floor.
  Conversely `ModularMode::Auto` must be **byte-identical** to an encoder that never named the knob
  — the guard that keeps the option unsent — and lossless + forced VarDCT must be the typed refusal.
- **jbrd — bit-exact reconstruction.** The vendored baseline-JPEG fixture recompresses and the
  libjxl oracle reconstructs the **original JPEG bytes exactly**; the stream's pixels also decode
  within the lossy agreement bound in both decoders; malformed/truncated JPEG inputs are typed
  errors that restore the output buffer.
- **Colour — signal pinned via oracle.** Every `ColorSpec` variant's structured encoding is read
  back field-by-field through `JxlDecoderGetColorAsEncodedProfile`; attached ICC bytes round-trip
  byte-exactly (and suppress the structured encoding); lossless stays bit-exact under every
  built-in spec; lossy XYB+PQ decodes back in the PQ domain at a sane PSNR.
- **Orientation — all eight exercised.** gamut and the oracle decode every EXIF orientation
  bit-identically (display-oriented); the four transposing values swap dimensions; Rotate180 is
  pinned by hand-reversal; explicit Identity is byte-identical to the default stream.
- **Metadata boxes.** Exact `Exif` (tiff-offset prefix included) and `xml ` box payloads pinned by
  a raw box scan; pixels stay bit-exact with boxes present; Codestream+metadata and empty payloads
  are typed errors.
- **Signatures / conversions.** Codestream vs. container signature bytes and the
  gray→RGB / RGBA→RGB / RGB→gray-rejection conversion contracts are pinned.
- **Mutants:** zero unjustified survivors; the only `exclude_re` entries carry strong justifications
  (encoder free choices whose output is unobservable through the decoders).
