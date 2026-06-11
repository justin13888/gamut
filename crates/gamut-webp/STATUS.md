# gamut-webp — implementation status

The complete component surface a conformant WebP encoder **and** decoder needs, drawn from the WebP
specs (RFC 9649 *WebP Image Format*; the Google *WebP Container*, *WebP Lossless Bitstream*, and
*Compression Techniques* references) and the VP8 spec (RFC 6386 *VP8 Data Format and Decoding
Guide*). Rows are **technical components**, not user features. This is the map for extension: each
module's doc comment cites the same spec section, and a row flips ☐→✅ (with the module
cross-reference) when it ships.

gamut is **image-first**: only the intra-frame / key-frame still-image subset of VP8 is in scope —
no inter-frame prediction, motion, or sequence coding. Unlike most of the workspace (encoder-first),
gamut-webp ships a **native decoder** too: the Rust ecosystem's WebP decoders all wrap libwebp,
whose memory-unsafety drove CVEs such as the zero-click CVE-2023-4863, so a `#![forbid(unsafe_code)]`
decoder is worth carrying. The in-crate decoder also doubles as the encoder's tier-2 oracle.

**Status:** ✅ = implemented · ☐ = planned (in scope) · ⊘ = out of scope (tracked for
container-completeness only). **Milestone (M)** is indicative sequencing, not a contract:

- **M0** — ✅ **done**: the RIFF/WebP container (read + write) and the minimal **VP8L lossless**
  still-image path (header, LSB bit I/O, canonical prefix codes, image data with the subtract-green
  transform), 8-bit RGB, simple file format (`RIFF`/`WEBP`/`VP8L`). `WebpEncoder::lossless` + native
  `WebpDecoder`. Verified bit-exact against libwebp (`libwebp-sys`) and against gamut's own decoder.
- **M1** — ✅ **done**: VP8L full — predictor / color / color-indexing transforms, LZ77 backward
  references, color cache, meta prefix codes — bit-exact lossless. The **decoder** reads the entire
  spec (any conformant stream); the **encoder** emits every feature with simple heuristics, leaving
  compression-density tuning (optimal mode/parse/clustering) to issue #31.
- **M2** — ✅ **done**: **VP8 lossy** key-frame intra — boolean entropy coder, frame header, intra
  prediction (16×16 DC/V/H/TM, per-4×4 B_PRED, chroma), Y2/WHT + integer 4×4 DCT, dequantization,
  zig-zag token coding, simple + normal loop filters, quantizer segmentation, 1/2/4/8 token
  partitions, per-macroblock skip. BT.601 YCbCr 4:2:0 added to gamut-color. `WebpEncoder::lossy`.
  **Bit-exact against libwebp in both directions** (gamut↔libwebp YUV) plus a malformed-input
  robustness corpus.
- **M3** — ✅ **done**: Extended container (encode + decode) — `VP8X` feature header + alpha (`ALPH`,
  raw and lossless), simple→extended promotion, RGBA API (`encode_rgba8` / `decode_to_rgba8`).
  libwebp recovers gamut's exact alpha and gamut recovers libwebp's. Alpha is a flagship still-image
  feature; `VP8X` is its enabler.
- **M4** — Color & metadata (**in scope**, embed on encode + preserve on decode): `ICCP` ICC
  profiles, `EXIF` / `XMP ` metadata, unknown-chunk round-trip preservation.
- **M5** — Animation: `ANIM` / `ANMF` — **out of scope** (decision 2026-06-09). Multi-frame
  sequences fall outside the image-first charter and the single-image `gamut_core` traits; WebP
  animation needs no codec work (each frame is an independent keyframe) but does need a non-trait
  multi-frame API. Rows are kept for container-completeness only: encode is not planned, decode may
  be revisited later. See [Scope decisions](#scope-decisions--non-core-feature-paths).
- **M6** — ✅ **done**: Decoder hardening — the libwebp interop matrix (both directions, including
  decoding cwebp's segmentation / per-segment filter / probability-update / compressed-alpha streams)
  and a malformed / truncated / exhaustive-bit-flip robustness corpus (no panics; typed errors).

The numbering of the two parts mirrors the two reference families: **Part 1** (sections A–F) is the
WebP container + VP8L lossless surface from `references/webp/`; **Part 2** (sections G–N) is the VP8
lossy-intra surface from `references/vp8/`. Section O is the cross-cutting API / oracle / tooling.

---

## Scope decisions — non-core feature paths

The core surface is the single still image (VP8L lossless + VP8 lossy intra, 8-bit RGB). Beyond
that, WebP's RIFF container carries optional chunks; this table records the **scope decision** for
each non-core path (in/out, and encode vs. decode), so the component tables below can be read against
a settled charter rather than a wish-list. `gamut-riff` already recognizes every FourCC involved
(`WebpChunkId`), so these are product-scope calls, not capability gaps.

| Feature path | Chunks | Decision | Encode | Decode | M | Rationale |
| --- | --- | --- | --- | --- | --- | --- |
| Alpha / transparency | VP8L native ARGB; `ALPH` + `VP8X` (lossy) | **In scope** | ✅ | ✅ | M3 | Flagship still-image feature (PNG-parity). VP8L alpha is free in ARGB; lossy alpha needs `ALPH` + `VP8X`. |
| Extended container | `VP8X` | **In scope** | ✅ | ✅ | M3 | Required enabler for lossy alpha, ICC, and metadata. Emitted only when a feature needs it (simple→extended promotion). |
| Color profile | `ICCP` | **In scope** | ✅ embed | ✅ preserve | M4 | Color correctness on wide-gamut images. |
| Metadata | `EXIF`, `XMP ` | **In scope** | ✅ embed | ✅ preserve | M4 | Cheap round-trip passthrough; preserved across decode→encode. |
| Animation | `ANIM`, `ANMF` | **Out of scope** (tracked only) | ✕ | deferred | M5 | Sequence content, against the image-first charter ("no video sequences") and the single-image `gamut_core` traits. Each `ANMF` frame is an independent keyframe — no codec work needed — but assembly requires a non-trait multi-frame API. Rows kept for container-completeness; a decode-only path may be revisited later. |

Markers: ✅ shipped · ✕ not planned · *deferred* = possible later, no commitment now.

---

# Part 1 — WebP container & VP8L lossless (`references/webp/`)

## A. Container / file format (RIFF · WebP — RFC 9649 §2; Google *WebP Container*)

Owner: [`gamut-riff`](../gamut-riff).

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| RIFF chunk: FourCC + `uint32` LE size + payload + pad-to-even | RFC 9649 §2.3 | ✅ | M0 |
| WebP file header: `RIFF`/`WEBP` form, back-patched file size | §2.4 | ✅ | M0 |
| chunk reader: iterate/validate chunks, bounds + padding | §2.3/§2.4 | ✅ | M0 |
| simple lossless: wrap `VP8L` payload | §2.6 | ✅ | M0 |
| simple lossy: wrap `VP8 ` payload (note trailing space) | §2.5 | ✅ | M0 |
| chunk routing: identify `VP8 `/`VP8L`/`VP8X` on read | §2.5–§2.7 | ✅ | M0 |
| `VP8X` extended header: feature flags + 24-bit canvas W/H (1-based) | §2.7 | ✅ | M3 |
| `ALPH` alpha chunk: preprocessing/filter/compression + bitstream | §2.7.1 (Alpha) | ✅ | M3 |
| simple→extended promotion (emit `VP8X` when a feature needs it) | §2.7 | ✅ | M3 |
| `ICCP` color profile chunk | §2.7.2 | ☐ | M4 |
| `EXIF` / `XMP ` metadata chunks | §2.7.3 | ☐ | M4 |
| chunk ordering enforcement (reconstruction chunks in canonical order) | §2.7 | ☐ | M4 |
| `ANIM` global animation parameters (bg color, loop count) | §2.7.1 (Animation) | ⊘ | M5 |
| `ANMF` per-frame chunk + frame disposal/blend, canvas assembly | §2.7.1 (Animation) | ⊘ | M5 |
| unknown-chunk passthrough (preserve order) | §2.7.4 | ☐ | M4 |

## B. VP8L bitstream header (RFC 9649 §3.4; Google *Lossless Bitstream*)

Owner: `gamut-webp/src/vp8l/header.rs`.

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| `0x2f` signature byte | RFC 9649 §3.4 | ✅ | M0 |
| 14-bit width-1 / 14-bit height-1 | §3.4 | ✅ | M0 |
| `alpha_is_used` hint (1 bit) | §3.4 | ✅ | M0 |
| version number (3 bits, must be 0) | §3.4 | ✅ | M0 |

## C. VP8L entropy coding / prefix codes (RFC 9649 §3.7)

Owner: `gamut-webp/src/vp8l/prefix.rs`.

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| canonical prefix (Huffman) code: build from code lengths | RFC 9649 §3.7.2 | ✅ | M0 |
| prefix-code decode (table-driven) — encoder oracle + native decode | §3.7.2 | ✅ | M0 |
| simple-code-length code (1–2 symbols) | §3.7.2 | ✅ | M0 |
| normal code: code-length code lengths + length-coded symbols | §3.7.2 | ✅ | M1 |
| prefix-code group: green+length / red / blue / alpha / distance (5 codes) | §3.7.1 | ✅ | M1 |
| meta prefix codes (entropy-image selecting per-block code groups) | §3.7.3 | ✅ | M1 |

## D. VP8L image data (RFC 9649 §3.6)

Owner: `gamut-webp/src/vp8l/{lz77,color_cache,encoder,decoder}.rs`.

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| literal ARGB pixel coding | RFC 9649 §3.6 | ✅ | M0 |
| scan-order pixel reconstruction | §3.6 | ✅ | M0 |
| LZ77 backward references: length/distance prefix codes | §3.6.2 | ✅ | M1 |
| distance mapping (2-D distance → plane code) | §3.6.2 | ✅ | M1 |
| color cache (hash of recent colors) | §3.6.3 | ✅ | M1 |

## E. VP8L transforms (RFC 9649 §3.5)

Owner: `gamut-webp/src/vp8l/transform.rs`. Transforms are emitted in order and inverted in reverse.

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| subtract-green transform (forward + inverse) | RFC 9649 §3.5 | ✅ | M0 |
| predictor (spatial) transform, 14 modes, block-size image | §3.5 | ✅ | M1 |
| color transform (green→red, green→blue, red→blue) | §3.5 | ✅ | M1 |
| color-indexing (palette) transform + index packing | §3.5 | ✅ | M1 |

## F. VP8L bit I/O (RFC 9649 §3.3)

Owner: `gamut-webp/src/vp8l/bit_io.rs`. **LSB-first**, diverging from `gamut-bitstream::BitWriter`
(MSB-first `f(n)`); a candidate to graduate into `gamut-bitstream` if a second consumer appears.

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| LSB-first `ReadBits(n)` reader | RFC 9649 §3.3 | ✅ | M0 |
| LSB-first bit writer (encoder side) | §3.3 | ✅ | M0 |

---

# Part 2 — VP8 lossy intra, key-frame only (`references/vp8/rfc6386.txt`)

## G. Boolean entropy coder (RFC 6386 §7)

Owner: `gamut-webp/src/vp8/bool_coder.rs`. Encoder + decoder; the decoder is production code and
also the encoder's hermetic round-trip oracle (cf. `gamut-bitstream/src/symbol.rs`). This is a
binary arithmetic coder, distinct from AV1's multi-symbol range coder.

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| `BoolDecoder`: 8-bit-probability interval decode (`get_bool`) | RFC 6386 §7.3 | ✅ | M2 |
| `BoolEncoder`: matching interval encode (`put_bool`) | §7.3 | ✅ | M2 |
| literal / signed-literal helpers (prob = 128) | §7.3 | ✅ | M2 |
| tree-coded symbol read/write (§8 tokenization over the bool coder) | §8 | ✅ | M2 |

## H. VP8 frame header (RFC 6386 §9)

Owner: `gamut-webp/src/vp8/header.rs`. Key-frame fields only.

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| uncompressed chunk: frame tag (type/version/show/part0 size) | RFC 6386 §9.1 | ✅ | M2 |
| key-frame start code `0x9d 0x01 0x2a` + 14-bit W/H + scale | §9.1 | ✅ | M2 |
| color space + clamping type | §9.2 | ✅ | M2 |
| segmentation: enable, map updates, per-segment quant/filter | §9.3, §10 | ✅ | M2 |
| loop-filter header: type, level, sharpness, deltas | §9.4 | ✅ | M2 |
| token-partition count + per-partition size offsets | §9.5 | ✅ | M2 |
| dequant indices: base + Y1/Y2/UV DC/AC deltas | §9.6 | ✅ | M2 |
| refresh golden/altref/last (key-frame forced) | §9.7, §9.8 | ✅ (n/a still) | M2 |
| DCT coefficient probability updates | §9.9, §13.4 | ✅ | M2 |
| `refresh_entropy_probs` (key-frame remainder) | §9.11 | ✅ | M2 |

## I. VP8 macroblock prediction records (RFC 6386 §11)

Owner: `gamut-webp/src/vp8/prediction.rs` (mode coding) + `vp8/tokens.rs` (mode prob tables).

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| `mb_skip_coeff` per macroblock | RFC 6386 §11.1 | ✅ | M2 |
| luma 16×16 mode: `DC`/`V`/`H`/`TM`/`B_PRED` | §11.2 | ✅ | M2 |
| B_PRED 4×4 subblock modes (10) + neighbor contexts | §11.2, §11.3 | ✅ | M2 |
| chroma 8×8 mode: `DC`/`V`/`H`/`TM` | §11.4 | ✅ | M2 |
| default key-frame mode probability tables | §11.5 | ✅ | M2 |

## J. VP8 intra prediction (RFC 6386 §12)

Owner: `gamut-webp/src/vp8/prediction.rs`.

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| chroma 8×8 prediction (`DC`/`V`/`H`/`TM`) | RFC 6386 §12.2 | ✅ | M2 |
| luma 16×16 prediction (`DC`/`V`/`H`/`TM`) | §12.3 | ✅ | M2 |
| luma 4×4 B_PRED prediction (10 directional/average modes) | §12.3 | ✅ | M2 |
| edge-pixel availability + clamping | §12.2, §12.3 | ✅ | M2 |

## K. VP8 coefficient / token coding (RFC 6386 §13)

Owner: `gamut-webp/src/vp8/tokens.rs`.

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| token tree: zero/one/.../cat1–6/EOB | RFC 6386 §13.2 | ✅ | M2 |
| value decode: extra bits + sign per category | §13.2 | ✅ | M2 |
| context selection (plane, band, neighbor non-zero) | §13.3 | ✅ | M2 |
| per-frame probability updates | §13.4 | ✅ | M2 |
| default coefficient probability tables | §13.5 | ✅ | M2 |

## L. VP8 dequant + inverse transforms + reconstruction (RFC 6386 §14)

Owner: `gamut-webp/src/vp8/transform.rs` + `vp8/quant.rs`. The 4×4 WHT here is **≠ AV1's WHT** in
`gamut-dsp`; VP8 transforms stay in-crate (no second consumer).

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| dequant factor tables (q-index → DC/AC, Y1/Y2/UV) | RFC 6386 §14.1 | ✅ | M2 |
| inverse 4×4 WHT for the Y2 (DC) block | §14.3 | ✅ | M2 |
| inverse 4×4 DCT for luma/chroma subblocks | §14.4 | ✅ | M2 |
| predict + residue summation + clamp to [0,255] | §14.5 | ✅ | M2 |
| forward 4×4 DCT + WHT + quantization (encoder) | §14 (encoder) | ✅ | M2 |

## M. VP8 loop filter (RFC 6386 §15)

Owner: `gamut-webp/src/vp8/loop_filter.rs`.

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| filter geometry: MB + subblock edges, raster order | RFC 6386 §15.1 | ✅ | M2 |
| simple filter | §15.2 | ✅ | M2 |
| normal filter (with high-edge-variance test) | §15.3 | ✅ | M2 |
| per-MB control-parameter derivation (level, limits, segments) | §15.4 | ✅ | M2 |

## N. Color / pixel formats

Owner: [`gamut-color`](../gamut-color). VP8L is RGB-native (no YCbCr); VP8 needs BT.601 4:2:0.

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| VP8L: 8-bit RGB / ARGB, identity (no color conversion) | — | ✅ | M0 |
| VP8: **limited-range** BT.601 RGB↔YCbCr + 4:2:0 chroma subsample/upsample (**new** module) | RFC 6386 §14.2; Google *WebP Container* (BT.601) | ✅ | M2 |
| alpha-channel plane handling | §2.7.1 (Alpha) | ✅ | M3 |

---

## O. Cross-crate API, oracle & tooling

Owner: [`gamut-webp`](.) + [`gamut-cli`](../gamut-cli).

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| `WebpEncoder` + `gamut_core::Encoder` impl (RGB8) | gamut-webp | ✅ | M0 |
| `WebpDecoder` + `gamut_core::Decoder` impl (→ RGB8) | gamut-webp | ✅ | M0 |
| `WebpConfig` / `WebpMode` native config | gamut-webp | ✅ | M0 |
| container parse + format routing (`VP8L` decoded; `VP8`/`VP8X` return `Unsupported`) | RFC 9649 §2 | ✅ | M0 |
| tier-1 oracle: internal forward/inverse round-trips (transforms, coders) | — | ✅ | M0 |
| tier-2 oracle: hermetic native decoder reproduces encoder output | — | ✅ | M0 |
| tier-3 oracle: `libwebp-sys` differential (enc→libwebp-dec, libwebp-enc→dec) | — | ✅ | M0 |
| CLI `gamut convert … .webp` (encode) + `.webp` decode input | gamut-cli | ✅ | M0 |
| wasm / ffi bindings for WebP | gamut-{wasm,ffi} | ☐ | future |
