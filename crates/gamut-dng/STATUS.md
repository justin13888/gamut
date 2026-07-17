# gamut-dng — DNG 1.7.1 implementation status

Tracking GitHub issue #109: implement a feature-complete **DNG (Digital Negative) 1.7.1** encoder
**and** decoder (`references/dng/DNG_Spec_1_7_1_0.pdf`). DNG is a TIFF/EP-based raw-image format,
so the container spine is the shared `gamut-ifd` primitive; this crate adds the DNG-specific tags,
raw photometry, compression, colour calibration, and metadata on top. Delivered as a stack of
small, individually-green phases (P1–P19) on the `feat/dng` branch; each is green
(`mise run test`/`lint`/`fmt-check`/`coverage` ≥80%).

**Keystone:** DNG's defining structure is an IFD *tree* — IFD0 (a preview/thumbnail) points, via
the `SubIFDs` tag (330), at the full-resolution raw image in a **sub-IFD**, with EXIF in another
(`ExifIFD` 34665). `gamut-ifd`'s writer only linked a flat IFD *chain*, so the first job (P2) is
sub-IFD **tree layout** — recursive two-pass absolute-offset assignment with pointer-tag patching —
added to `gamut-ifd` (which `gamut-exif`, issue #34, will share). Once an uncompressed CFA DNG
validates clean against the Adobe SDK (P4–P6), each later phase swaps a compression scheme,
photometry, or metadata block into the same spine.

**Oracle:** the authoritative **Adobe DNG SDK 1.7.1** (`references/dng/`), built headless via the
`cc` crate into the dev-only `tooling/gamut-dng-oracle` (XMP disabled; system zlib; a libjxl shim
since JXL is out of baseline scope), exposing `extern "C"` validate / parse-negative / render
entry points around `dng_validate`'s call sequence. gamut-encode → `dng_validate` must accept the
file; Adobe `sample_files/*.dng` → gamut-decode must match. A DNG is also a valid TIFF, so the
existing `libtiff-oracle` cross-checks the container/strips **pixel-exactly**, and internal
encode→decode round-trips guard every lossless path.

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
| P9  | Ch5     | Levels (Black/White) + ActiveArea + DefaultCrop + **bit-depth packing 8/10/12/14/16** (MSB-first, Adobe-verified pixel-exact). Completed by #253: the full spec level model (`RawLevels`: BlackLevelRepeatDim pattern, RATIONAL blacks, DeltaH/V, per-plane WhiteLevel, LinearizationTable, MaskedAreas) and the chapter-5 mapping itself (`RawImage::to_linear`, gated ±1 LSB against the Adobe SDK's stage-2 image) | ✅ done |
| P10 | Ch2     | Embedded uncompressed RGB preview in IFD 0 (JPEG preview + size cap deferred) | ✅ done |
| P11 | Ch2–5   | **Decoder**: walk the tree (SubIFDs → raw), unpack samples, reconstruct RawImage + CameraProfile; round-trips & agrees with Adobe | ✅ done |
| P12 | Ch4     | Deflate/ZIP (8) encode+decode (`miniz_oxide`, zlib format) — CFA + LinearRaw, Adobe-validated | ✅ done |
| P13 | Ch4     | Lossless JPEG (7) encode+decode (SOF3) — CFA + LinearRaw, Adobe decodes pixel-exact. #253 hardened decode to the full T.81 process-14 reader envelope (predictors 1–7, point transform, per-component tables, restarts; SDK-differential) and published the `lossless_jpeg` module | ✅ done |
| P14 | Ch2     | Tiled raw layout (`TileOffsets`/`TileByteCounts`) | ⏳ deferred |
| P15 | Ch2     | BigTIFF DNG (1.7, 64-bit offsets) — encode + decode, Adobe-validated | ✅ done |
| P16 | Ch8–9   | Metadata: EXIF sub-IFD + XMP (700) / IPTC (33723) / ICC (34675) — embed + decode, Adobe-validated | ✅ done |
| P17 | Ch2     | Digests: MD5 `NewRawImageDigest`/`RawImageDigest`/`RawDataUniqueID` | ⏳ deferred |
| P18 | Ch7     | `OpcodeList1/2/3` container + standard opcode library. Container done via #253: typed `OpcodeList`/`Opcode` parse + pass-through write + `DNGBackwardVersion` raising; the standard opcode *processing* library remains deferred | ◑ partial |
| P19 | —       | Finalization: docs + top-to-bottom API review (CLI N/A — DNG needs raw sensor input, not the CLI's processed PNG/JPEG inputs) | ✅ done |

## Bridge surface for external RAW pipelines (issue #253)

Downstream raw *processors* (e.g. rawshift) consume gamut-dng's decode as their DNG front end and
run their own develop pipeline on top. #253 completed the standard-compliant surface they bridge
to: the typed `RawLevels` model (P9), the chapter-5 `RawImage::to_linear` mapping (stage-2
oracle-gated, so downstreams call it instead of reimplementing the spec), typed opcode-list
containers (P18, processing still ours to do later), and the hardened, now-public
`lossless_jpeg` module. Opaque tag blobs are deliberately **not** exposed — the crate parses
spec structures into typed values and writes them back.

## Deferred to follow-up campaigns

The baseline (P1–P16) ships the full encode+decode pipeline — container/sub-IFD tree, CFA +
LinearRaw, uncompressed / Deflate / lossless-JPEG compression, the colour-calibration profile,
8/10/12/14/16-bit packing, embedded preview, EXIF/XMP/IPTC/ICC metadata, and BigTIFF — all
Adobe-validated. The remaining items each plug into the same IFD-tree / strip pipeline and the
Adobe + libtiff oracles the same way every phase above does:

- **Tiled layout** (P14) — `TileOffsets`/`TileByteCounts`; the strip pipeline already covers every
  pixel mode, and the `ImageBlocks` writer already lays out N blocks, so tiling is a layout swap
  (tile/untile with edge padding) plus the decoder's tile reassembly.
- **Raw digests** (P17) — `NewRawImageDigest`/`RawImageDigest` must bit-match the SDK's internal
  MD5-over-image algorithm; verifying that needs a `qDNGValidate=1` oracle build (the current build
  suppresses digest-mismatch reporting). `RawDataUniqueID` is an opaque MD5 the SDK does not verify.
- **JPEG XL compression** (`Compression = 52546`, DNG 1.7) — **planned**: decode depends on the
  tiled layout (P14) landing first (real JXL DNGs, e.g. ProRAW-adjacent files, are tiled) plus a
  `gamut-jxl` decode path; encode additionally needs the `gamut-jxl` (libjxl) encoder. The oracle
  already links/stubs libjxl so it can read Adobe's JXL sample DNGs.
- **Lossy JPEG** (`Compression = 34892`) — skipped as low-value; needs a baseline DCT codec
  (`gamut-tiff` likewise deferred JPEG-in-TIFF).
- **The standard opcode processing library** (P18) — executing the parsed opcodes:
  `WarpRectilinear`/`WarpFisheye`, `FixVignetteRadial`, `FixBadPixelsConstant`/`List`, `TrimBounds`,
  `MapTable`/`MapPolynomial`, `GainMap`, `DeltaPerRow`/`Col`, `ScalePerRow`/`Col`,
  `WarpRectilinear2`. The container/typed lists already round-trip (#253).
- **Advanced image types** — transparency masks, depth maps, semantic masks, and the enhanced
  image (`NewSubFileType` 4/8/16/0x10004); `ProfileGainTableMap`, `RGBTables`, `ImageStats`,
  `ImageSequenceInfo`, C2PA manifest.
- **Floating-point samples** (`SampleFormat = 3`) and the DNG float predictors (34894/34895).
