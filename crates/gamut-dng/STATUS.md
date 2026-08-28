# gamut-dng — DNG 1.7.1 implementation status

Tracking GitHub issue #109: a feature-complete **DNG (Digital Negative) 1.7.1** encoder **and**
decoder (`references/dng/DNG_Spec_1_7_1_0.pdf`). DNG is a TIFF/EP-based raw-image format, so the
container spine is the shared `gamut-ifd` primitive; this crate adds the DNG-specific tags, raw
photometry, compression, colour calibration, and metadata on top. Delivered as a stack of small,
individually-green phases; each is green (`mise run test`/`lint`/`fmt-check`/`coverage` ≥80%).

**Keystone:** DNG's defining structure is an IFD *tree* — IFD0 (a preview/thumbnail) points, via
the `SubIFDs` tag (330), at the full-resolution raw image in a **sub-IFD**, with EXIF in another
(`ExifIFD` 34665). `gamut-ifd`'s writer only linked a flat IFD *chain*, so the first job (P2) was
sub-IFD **tree layout** — recursive two-pass absolute-offset assignment with pointer-tag patching.

**Oracle:** the authoritative **Adobe DNG SDK 1.7.1** (`references/dng/`), built headless via the
`cc` crate into the dev-only `tooling/gamut-dng-oracle` (XMP stubbed; system zlib; **real libjxl
0.12.0** statically linked via `gamut-jxl-sys`, so the SDK genuinely decodes JPEG XL DNGs),
exposing `extern "C"` validate / read-stage-1 / read-stage-2 / digest entry points around
`dng_validate`'s call sequence, plus the SDK ZIP's 14 official `sample_files/*.dng` as
decode-conformance inputs. gamut-encode → `dng_validate` must accept the file (raw digest
included); Adobe sample DNGs → gamut-decode must agree with the SDK. A DNG is also a valid TIFF,
so the existing `libtiff-oracle` cross-checks the container/strips **pixel-exactly**, and
internal encode→decode round-trips guard every lossless path.

## Phases

| Phase | DNG § | Scope | Status |
| ----- | ----- | ----- | ------ |
| P1  | —       | Scaffold: crate, workspace + umbrella wiring, README, region-free skeleton | ✅ done |
| P2  | —       | **Keystone** `gamut-ifd`: sub-IFD tree writer + pointer patching + `read_ifd_at` | ✅ done |
| P3  | Ch3     | DNG tag + value tables (`tags`, `values`) from the SDK headers | ✅ done |
| P4  | Ch2–5   | **Keystone** uncompressed CFA DNG: IFD0 preview + raw sub-IFD, mandatory tags, strips, II/MM | ✅ done |
| P5  | —       | `tooling/gamut-dng-oracle`: auto-extract + `cc`-build SDK + `extern "C"` shim | ✅ done |
| P6  | —       | Adobe oracle gate on: gamut-encode → `dng_validate`; libtiff IFD-0 cross-check | ✅ done |
| P7  | Ch4     | `LinearRaw` photometric (demosaiced RGB), samples-per-pixel / photometric handling | ✅ done |
| P8  | Ch6     | Colour & calibration: ColorMatrix1/2, CameraCalibration, ForwardMatrix, illuminants, AnalogBalance, BaselineExposure, profile name/policy + `CameraProfile` API | ✅ done |
| P9  | Ch5     | Levels (Black/White) + ActiveArea + DefaultCrop + **bit-depth packing 8/10/12/14/16** (MSB-first, Adobe-verified pixel-exact). Completed by #253: the full spec level model (`RawLevels`) and the chapter-5 mapping itself (`RawImage::to_linear`, gated ±1 LSB against the Adobe SDK's stage-2 image) | ✅ done |
| P10 | Ch2     | Embedded uncompressed RGB preview in IFD 0 (JPEG preview + size cap deferred) | ✅ done |
| P11 | Ch2–5   | **Decoder**: walk the tree (SubIFDs → raw), unpack samples, reconstruct RawImage + CameraProfile; round-trips & agrees with Adobe. Finalization hardened it: full IFD-forest raw search, per-strip sub-byte alignment, `SampleFormat` validation, 64-bit offset reads | ✅ done |
| P12 | Ch4     | Deflate/ZIP (8) encode+decode (zlib format; encode `gamut-deflate`, decode `miniz_oxide`) — CFA + LinearRaw, Adobe-validated; encode limited to 8/16-bit (the SDK reader's constraint) | ✅ done |
| P13 | Ch4     | Lossless JPEG (7) encode+decode (SOF3) — CFA + LinearRaw, Adobe decodes pixel-exact. #253 hardened decode to the full T.81 process-14 reader envelope and published the `lossless_jpeg` module; per-chunk geometry follows the spec's total-sample-count rule | ✅ done |
| P14 | Ch2     | Tiled raw layout (`TileWidth`/`TileLength`/`TileOffsets`/`TileByteCounts`): decode with edge-crop reassembly + `with_tiling` encode (zero-padded edge tiles), all schemes, Adobe pixel-exact | ✅ done |
| P15 | Ch2     | BigTIFF DNG (1.7, 64-bit offsets) — encode + decode, Adobe-validated | ✅ done |
| P16 | Ch8–9   | Metadata: EXIF sub-IFD + XMP (700) / IPTC (33723) / ICC (34675) — embed + decode, Adobe-validated | ✅ done |
| P17 | Ch2     | Digests: the encoder writes `NewRawImageDigest` (51111), bit-matching the SDK's `FindNewRawImageDigest` (256×256 digest tiles, planar LE serialisation, the ≤256-entry-table byte mode); `RawImage::new_raw_image_digest` is public and the decoder surfaces the stored digest | ✅ done |
| P18 | Ch7     | `OpcodeList1/2/3` container + standard opcode library. Container done via #253: typed `OpcodeList`/`Opcode` parse + pass-through write + `DNGBackwardVersion` raising; the standard opcode *processing* library remains deferred | ◑ partial |
| P19 | —       | Finalization: JPEG XL, sub-images, gain maps, extra-tag explicitness, version auto-computation, docs + API freeze (v1.0.0) | ✅ done |
| P20 | Ch3–4   | **JPEG XL** (Compression 52546, DNG 1.7): decode always available (pure-Rust jxl-rs; bare codestream + container, full-range 16-bit per the SDK's semantics, fp16 rejected typed); encode behind the `jxl-encode` feature (libjxl; lossless or lossy, `JXLDistance`/`JXLEffort` written); `RowInterleaveFactor`/`ColumnInterleaveFactor` de-interleave on decode | ✅ done |
| P21 | Ch2,4   | **Sub-images**: every non-raw image IFD as a typed `SubImage` (previews, transparency masks, **semantic masks** with `SemanticName`/`SemanticInstanceID`/`MaskSubArea`, depth maps + `DepthInfo`), best-effort decoded with verbatim-chunk fallback | ✅ done |
| P22 | Ch4     | **Gain maps**: typed `ProfileGainTableMap` (52525) + `ProfileGainTableMap2` (52544) — parse, byte-exact re-serialise, embed on encode; gated against Adobe's PGTM sample files | ✅ done |
| P23 | —       | **Explicitness**: every unmodelled IFD field surfaces verbatim as a typed `RawTag` (`ifd0_extra`/`raw_extra`/`exif_extra`/per-sub-image), via a consumption-tracking reader — issue #109's "all metadata explicitly represented" clause | ✅ done |
| P24 | Ch2-4   | **Real camera conformance** (#174): the `gamut-dng-samples` corpus + `tooling/gamut-dng-real-conformance`; `Predictor`/`PlanarConfiguration` honoured, byte accounting and the preserving rewrite fixed for real files, optional camera profile | ✅ done |

## Apple ProRAW (DNG 1.7 + JPEG XL): fully covered for decode

A ProRAW-with-JXL DNG (iPhone 15/16 Pro era) is a DNG 1.7.0.0 linear DNG. Every ingredient maps
to a shipped, oracle-gated feature:

| ProRAW ingredient | Coverage |
| ----------------- | -------- |
| DNG 1.7.0.0 / backward 1.3+ | version parsing + typed `backward_version` (P11/P23) |
| `LinearRaw`, 3 samples/pixel | P7 |
| Tiled layout (e.g. 2016×2016) | P14 |
| Compression 52546 (JPEG XL), bare codestreams | P20 — full-range 16-bit decode, matching the reference SDK (Apple pairs `BitsPerSample = 10` with `WhiteLevel = 65535`) |
| `LinearizationTable`, per-plane Black/WhiteLevel | P9 |
| Semantic-mask sub-IFDs (PhotometricMask 52527, JXL) | P21 (+P20) |
| `ProfileGainTableMap` | P22 |
| JPEG preview in IFD 0 | P21 (decoded when in scope, verbatim chunks otherwise) |
| Apple maker tags | P23 (verbatim typed `RawTag`s) |
| `NewRawImageDigest` | P17 |

Conformance uses Adobe's official JPEG XL sample DNGs (tiled, interleaved, lossy) — gamut's
decode agrees with the SDK's own real-libjxl decode within one code (JXL conformance tolerance
for lossy streams; lossless is bit-exact) — plus ProRAW-shaped synthetic goldens through the
full encode → SDK-validate → decode → digest loop. Since #174 a **real iPhone 12 Pro ProRAW
file** is gated too (below), which is what turned the table above from a mapping argument into a
measurement.

## Real camera conformance (issue #174)

Every input above is either synthetic or Adobe-authored. **Real camera files are a different
population**, and running six of them through the decoder found four defects nothing else could
have: two produced silently wrong output, one dropped 651 KB, one refused a valid file outright.

**Corpus:** the `gamut-dng-samples` submodule at `third_party/gamut-dng-samples` — six CC0 files
from raw.pixls.us, each verified CC0 by SHA-256 against the upstream index, kept byte-identical
to upstream (cropping would destroy the byte-completeness and digest properties they exist to
test). `MANIFEST.toml` carries provenance plus *measured* expectations, so drift fails rather
than passes quietly.

**Harness:** `tooling/gamut-dng-real-conformance`, excluded from the workspace so
`cargo test --workspace` never pulls in ~178 MiB of camera files. Run it with `mise run
fetch-dng-samples && mise run test-dng-real`; CI runs it in `extended.yml`'s `real-dng` job, not
on the per-PR path. Five layers per file: byte accounting (including the exact inventory of
unaccounted runs), decode against the manifest, the stored digest under the storage-correct rule,
the Adobe SDK stage-2 differential at ±1 code, and the preserving rewrite.

| File | What it alone proves |
| ---- | -------------------- |
| Apple iPhone 12 Pro (ProRAW) | Big-endian, tiled 504×378 12-bit `LinearRaw`, `LinearizationTable`, PGTM, semantic mask, a real MakerNote pinned across a rewrite — and a 10-byte `APPLEDNG` vendor preamble |
| Canon 5D3 uncompressed | `Compression = 1`, one 5920×3950 16-bit CFA strip |
| Canon 5D3 lossless | `Compression = 7` CFA, 384 tiles |
| Canon 5D3 lossy | `Compression = 34892`, a deferral that must be a *typed* refusal; the only file whose digest uses the compressed-chunk rule, so its integrity verifies even though its image does not decode |
| Leica M Monochrom | DNG 1.0.0.0 monochrome carrying **no colour calibration at all** — must decode with no profile rather than fail |
| Leica M10 | Raw in IFD 0 itself, previews with no `RowsPerStrip`, and a **651 KB appended trailer** the rewrite must carry |

What the corpus fixed:

- **`Predictor` (317)** was parsed and then ignored — a `Predictor = 2` file decoded to garbage
  with no error. Now undone per chunk following the SDK's `DecodeDelta8/16/32` exactly (rows
  independent, back-reference `samples_per_pixel × x_factor`, wrapping at the *container* width),
  with the float predictors and sub-byte depths refused typed. Self-predicting schemes (lossless
  JPEG, JPEG XL) ignore the tag, as the SDK reader does.
- **`PlanarConfiguration` (284)** was never read, so planar storage would have been misread as
  chunky. Now validated: chunky accepted, planar refused typed.
- **Byte accounting** did not hold for real files. `gamut-ifd` gained `SpanKind::{Preamble,
  Interstitial, Trailer}` and an explicit `classify_unclaimed` pass, so every byte of every real
  file classifies *and* the report still says what each run was. The pass is skipped when the walk
  admits a `SkippedSubIfd`, so it can never mask a parser defect, and the dual-ledger invariants
  are untouched.
- **`DngRewrite` dropped those bytes.** It now carries every unaccounted run through verbatim and
  reports each in `RewrittenDng::preserved`. Bytes survive; original absolute offsets generally do
  not, because the directory layout is rebuilt — the runs are appended after the payload region in
  file order, which leaves a trailer last.

## v2.0.0: what changed and why

Three breaking changes, all forced by real files:

- **`DecodedDng::profile` is `Option<CameraProfile>`.** A monochrome camera has no colour to
  calibrate and legitimately writes no `ColorMatrix1`, `CalibrationIlluminant1` or
  `AsShotNeutral`; the Leica M Monochrom does exactly that and previously failed the whole decode.
  Absent calibration now yields `None` — nothing is invented. Calibration that is *present and
  malformed* is still an error.
- **`PhotometricInterpretation::YCbCr` (6)** is modelled, so the baseline-JPEG previews every real
  camera embeds stop raising an anomaly per preview.
- **`DngDecoder::verify_new_raw_image_digest`** is new, returning `DigestCheck`. It picks the rule
  the file's storage demands — sample-domain for lossless, compressed-chunk for lossy/JXL — which
  a caller could not previously do, because `lossy_compressed_digest` is crate-private.

`AsShotWhiteXY` (50729) as a typed alternative to `AsShotNeutral` remains **deferred**: no corpus
file needs it, converting xy to camera neutral is DNG §6 rendering work, and the tag surfaces
verbatim through `ifd0_extra` meanwhile.

## Bridge surface for external RAW pipelines (issue #253)

Downstream raw *processors* (e.g. rawshift) consume gamut-dng's decode as their DNG front end and
run their own develop pipeline on top. #253 completed the standard-compliant surface they bridge
to: the typed `RawLevels` model (P9), the chapter-5 `RawImage::to_linear` mapping (stage-2
oracle-gated, so downstreams call it instead of reimplementing the spec), typed opcode-list
containers (P18, processing still ours to do later), and the hardened, now-public
`lossless_jpeg` module. The typed encode/decode path deliberately exposes no opaque tag blobs —
it parses spec structures into typed values and writes them back; **preservation** is a
separate path (below).

## Byte completeness and the preserving rewrite (issue #263)

#263 verified — and, where verification failed, fixed — the byte-completeness story end to end:

- **`deconstruct`** is rebuilt on `gamut_ifd::audit`'s dual-ledger engine: every byte of the
  file classifies into typed segments (u64-native, so >4 GiB BigTIFF strips no longer
  false-flag), embedded camera-profile streams (`ExtraCameraProfiles` → `.dcp`-form,
  magic `0x4352`, stream-relative offsets) are walked and claimed at physical positions, and
  the strict verdict is the zero-tolerance `SegmentReport::is_fully_classified`. Gated over the
  Adobe SDK's full `sample_files` corpus (`tests/corpus.rs`): all fourteen Adobe-authored DNGs
  — JXL tiles, PGTM2, ImageStats, ImageSequenceInfo, HDR/SDR profiles — classify to the last
  byte with the parser cross-check holding.
- **`DngRewrite`** is the preservation path the typed codec deliberately is not: open the whole
  tree losslessly (unknown/vendor tags and unknown field types survive as data), edit it
  surgically, and write it back with every tag value byte-exact, every strip/tile/embedded-JPEG
  payload copied verbatim (never re-encoded), and the `MakerNote` **pinned at its original
  absolute offset** whenever the new layout permits (`MakerNotePreservation` reports the
  outcome). Intentional drops, in full: declared dead space (`FreeOffsets`/`FreeByteCounts` —
  the tags name explicitly-dead bytes) is dropped; a file carrying `ExtraCameraProfiles` is
  refused (`Unsupported`, deferred) rather than rewritten lossily. Corpus-gated: every
  rewritable Adobe sample survives open → write fully classified, with its unknown-tag
  inventory unchanged and `dng_validate` accepting the result wherever it accepts the original.

## v1.0.0 freeze decisions

- Spec-coded enums (`Compression`, `PhotometricInterpretation`, `CalibrationIlluminant`,
  `CfaLayout`, `Predictor`, `SampleFormat`, `ProfileEmbedPolicy`, `PreviewColorSpace`,
  `SubImageKind`), report enums (`Severity`, `Anomaly`), `RawPhotometry`, and decoder-output
  structs (`DecodedDng`, `SubImage`, `LinearImage`, `LosslessJpeg`, `DeconstructReport`,
  `UnknownTag`, `SemanticMaskInfo`, `DepthInfo`, `SubImageData`) are `#[non_exhaustive]` —
  future spec codes and fields are additive.
- Encoder-input data structs (`DngMetadata`, `ExifMetadata`, `Opcode`, `ProfileGainTableMap`,
  `RawTag`, `MaskSubArea`) keep literal construction (no `non_exhaustive`); adding a field there
  is accepted as semver-major. `GainValues` stays exhaustive — its four variants are the spec's
  closed `DataType` set.
- Re-export closure: everything on the crate root, including `RawPhotometry`, `cfa_color`,
  `opcode_id`, `new_subfile_type`, and `gamut_ifd::Value` (the `RawTag` payload type);
  `lossless_jpeg::{encode, decode}` stay module-scoped deliberately (a codec namespace).
- `Compression::is_supported` became `is_decodable` (every decodable scheme encodes, with the
  documented `jxl-encode`/Deflate-depth caveats).
- JPEG XL range semantics are frozen to the reference SDK's: decoded JXL data is full-range
  16-bit; a JXL IFD's `BitsPerSample` records codestream precision; encode requires 16-bit
  input.

## Deflate codec choice (#196)

Encode uses `gamut-deflate` at `Level::Default`; decode uses `miniz_oxide`, bounded to the packed
length the chunk geometry implies. `gamut-deflate` is deliberately encoder-only (inflating is
solved and security-sensitive), so the split is permanent, not a staging post — the same one
`gamut-tiff` and `gamut-png` make.

Measured by `cargo bench -p gamut-dng --bench compression`, against the `miniz_oxide` level 6 this
crate encoded with before:

- **Ratio is a wash.** On packed raw payloads `Level::Default` lands within ±0.6% of miniz-6 —
  slightly better tiled, slightly worse as one strip. Raw sensor noise leaves DEFLATE modelling
  almost nothing to work with (a 16-bit frame compresses ~3%), so the entropy-coding differences
  that separate these encoders on text do not show up here.
- **Encode is ~17% slower** (≈46 MB/s vs ≈56 MB/s); `Level::Best` is ~12× slower again.
- **`Level::Best` only pays off tiled** (−0.3% to −0.5% on real raw, −5.5% on 8-bit), because
  `gamut-deflate` applies its optimal parse at 1 MiB or below and the untiled encoder writes
  `RowsPerStrip = ImageLength` — one strip for the whole image, above the threshold. Raising that
  limit is tracked upstream rather than worked around here, which is why the shipped level stays
  `Default`.

The Adobe DNG SDK validates the output on every fixture the oracle covers (CFA and LinearRaw,
8- and 16-bit, strips and tiles), so the migration is correctness-neutral.

## Deferred / out of scope

Each deferred item plugs into the same IFD-tree/chunk pipeline and oracles the shipped features
use; additions are semver-additive.

- **Lossy JPEG** (`Compression = 34892`) — needs a baseline DCT codec (`gamut-tiff` likewise
  deferred JPEG-in-TIFF). Decode surfaces such images as verbatim chunks today; a lossy *raw*
  IFD is refused with a typed `Unsupported`, and its `NewRawImageDigest` still verifies (the
  compressed-chunk rule needs no pixels).
- **`AsShotWhiteXY`** (50729) as a typed alternative to `AsShotNeutral` — the tag surfaces
  verbatim through `ifd0_extra`; converting xy to camera neutral is DNG §6 rendering work and no
  real file in the corpus needs it. A file carrying only `AsShotWhiteXY` decodes with no profile.
- **Restoring *interior* unaccounted bytes to their original offsets** — #350 landed the leading
  case: a vendor preamble now keeps its offset, because `gamut-ifd`'s writer reserves the
  header/first-directory gap for it (`WriteOptions::with_preamble`), which is the position that
  matters — Apple ProRAW's `APPLEDNG` sits there and a vendor tool looks for it there. Interstitial
  filler and an appended trailer still keep only their bytes: an interstitial run's original
  position is interior to a payload layout the rewrite does not reproduce (the strips it sat
  between are re-packed), so there is no offset to restore it to. `PreservedSpan` reports both
  offsets, so a caller can see which happened.
- **Floating-point samples** (`SampleFormat = 3`, fp16 JPEG XL, the float predictors
  34894/34895) — rejected with typed errors on decode; the u16 sample model would need a float
  sibling.
- **The standard opcode processing library** (P18) — *executing* `WarpRectilinear`, `GainMap`,
  `FixVignetteRadial`, … The typed containers round-trip; processing is a raw-developer concern.
- **Applying gain maps / rendering** — `ProfileGainTableMap` parses typed (and `gain_at`
  decodes entries); applying it in RIMM space is rendering-pipeline work.
- **Writing mask/depth/enhanced sub-IFDs** — decode-only today; the encoder writes the raw +
  preview tree.
- **4-colour CFA encoding** (RGBW/CYGM) — `CameraProfile` is a 3×3 model; widening it (4×3
  matrices, `ReductionMatrix`) is its own feature. Decode of 4-colour patterns works.
- **PGTM2 inside Camera Profile IFDs** (`ExtraCameraProfiles`) — surfaced via extras; typed
  parse covers the IFD0/raw-IFD placements.
- **JPEG-compressed previews as pixels** — surfaced as verbatim chunks (`SubImageData::
  Undecoded`); decoding them needs the baseline DCT codec above.
- **Advanced 1.7 metadata without a typed surface** (`RGBTables`, `ImageStats`,
  `ImageSequenceInfo`, `ProfileDynamicRange`, C2PA) — explicitly surfaced as typed `RawTag`s.
- **Pluggable codestream backends** (#241) — no hardware acceleration exists for the DNG
  compression schemes (Uncompressed/Deflate/lossless-JPEG/JPEG XL); gamut's software
  implementation is always used, so no backend seam is exposed.
