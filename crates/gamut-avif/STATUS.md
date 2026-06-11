# gamut-avif — implementation status

The complete component surface a conformant AVIF encoder needs, drawn from every related spec
(AV1 Bitstream & Decoding Process Specification; AVIF v1.2.0; AV1-ISOBMFF v1.3.0; ISO/IEC 14496-12
ISOBMFF; 23008-12 HEIF; 23000-22 MIAF; ITU-T H.273 CICP). Rows are **technical components**, not
user features. This is the map for extension: each module's doc comment cites the same spec
sections, and a row flips ☐→✅ (with the module cross-reference) when it ships.

**Status:** ✅ = implemented (milestone M0) · ☐ = planned. **Milestone** is indicative sequencing,
not a contract:

- **M0** — MVP (current): lossless intra, identity `mc=0`, 4:4:4, 8-bit, full range, single tile,
  64×64 superblocks, `DC_PRED`, forced `TX_4X4` Walsh–Hadamard, static default CDFs
  (`disable_cdf_update = 1`). Verified bit-exact against vendored `libavif`/`dav1d`.
- **M1** — Lossy intra: forward DCT/ADST + quantization + RD/rate control, CDF adaptation, full
  intra mode set, variable tx size/type, multi-tile, in-loop filters, 128×128 SB, full partition
  set, segmentation/delta-q, superres, screen-content tools (palette/intrabc).
- **M2** — Pixel formats: 10/12-bit, 4:2:0/4:2:2, monochrome, profiles 0 & 2, limited range,
  RGB↔YCbCr + chroma resample, `MA1B` baseline brand.
- **M3** — Alpha & auxiliary: alpha aux item, `auxC`/`auxl`, premultiplied (`prem`), depth maps.
- **M4** — Color & metadata: ICC profiles, Exif/XMP items, HDR (PQ/HLG, `mdcv`/`clli`), film grain.
- **M5** — Container transforms & derivation: `irot`/`imir`/`clap`/`pasp`, `grid`/overlay,
  thumbnails, `idat`, `iloc` v1/v2.
- **M6** — Image sequences: `avis` brand, ISOBMFF tracks, full AV1 inter-coding machinery.

## A. Container / file format (ISOBMFF · HEIF · MIAF · AVIF · AV1-ISOBMFF)

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| `ftyp`: major `avif`, compat `avif`/`mif1`/`miaf`/`MA1A` | AVIF §6,§8.3 | ✅ | M0 |
| `MA1B` baseline brand (Main profile, ≤L5.1, 4:2:0) | AVIF §8.2 | ☐ | M2 |
| `avis` brand (image sequences) | AVIF §7 | ☐ | M6 |
| `meta` (FullBox v0) container | 14496-12 | ✅ | M0 |
| `hdlr` handler_type=`pict` | 23008-12 | ✅ | M0 |
| `pitm` primary item id | 14496-12 | ✅ | M0 |
| `iloc` v0, construction_method=0 → `mdat`, 4-byte `extent_offset` back-patch | 14496-12 | ✅ | M0 |
| `iloc` v1/v2, construction_method=1 (`idat`)/2 (item) | 14496-12 | ☐ | M5 |
| `iinf`+`infe` v2, item_type=`av01` | 14496-12 | ✅ | M0 |
| `iprp`/`ipco`/`ipma` property association (`av1C` essential) | 14496-12; AVIF §2.2.1 | ✅ | M0 |
| `av1C` AV1ItemConfigurationProperty, empty `configOBUs` | AV1-ISOBMFF §2.3 | ✅ | M0 |
| `ispe` image spatial extents | 23008-12 | ✅ | M0 |
| `pixi` pixel information (3×8) | 23008-12 | ✅ | M0 |
| `colr` type `nclx` (CICP code points) | AVIF §2.2; AV1-ISOBMFF §2.3.4 | ✅ | M0 |
| `colr` type `rICC`/`prof` (ICC profile) | 23008-12 | ☐ | M4 |
| `pasp` pixel aspect ratio | 14496-12 | ☐ | M5 |
| `clap` clean aperture | 23008-12 | ☐ | M5 |
| `irot` rotation / `imir` mirror | 23008-12 | ☐ | M5 |
| `auxC` aux-type property + `auxl` item ref (alpha plane) | 23008-12; AVIF §4 | ☐ | M3 |
| `prem` premultiplied-alpha association | AVIF §4 | ☐ | M3 |
| `iref` (`auxl`/`dimg`/`thmb`/`cdsc`) | 23008-12 | ☐ | M3/M5 |
| `grid` derived item + `dimg` refs (tiled mosaic) | 23008-12; MIAF | ☐ | M5 |
| `idat` inline item data | 14496-12 | ☐ | M5 |
| `thmb` thumbnail item | 23008-12 | ☐ | M5 |
| Exif / XMP metadata items + `cdsc` ref | AVIF §3; 23008-12 | ☐ | M4 |
| `a1op` operating-point sel / `a1lx` layered index / `lsel` layer sel | AVIF §2.2 | ☐ | M6 |
| sequence tracks: `moov`/`trak`/`mdia`/`stbl`, `av01` sample entry, `av1C` in `stsd` | 14496-12; AV1-ISOBMFF §3 | ☐ | M6 |
| `mdat` payload = AV1 temporal unit OBUs | AV1-ISOBMFF §2.4 | ✅ | M0 |
| cross-box consistency (av1C↔seq-hdr, `pixi`, `colr` range, `ispe` dims) | AVIF §2.2/§2.3.4 | ✅ | M0 |

## B. AV1 — OBUs, sequence & frame headers

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| OBU header + `obu_has_size_field`=1 + LEB128 size | §5.3,§4.10.5 | ✅ | M0 |
| `OBU_SEQUENCE_HEADER` | §5.5 | ✅ | M0 |
| `OBU_FRAME` (frame header ∥ tile group) | §5.10 | ✅ | M0 |
| `OBU_FRAME_HEADER` + separate `OBU_TILE_GROUP` | §5.9/§5.11 | ☐ | M1 |
| `OBU_TEMPORAL_DELIMITER` (omitted in AVIF item) | §5.6; AV1-ISOBMFF §2.4 | ✅ (omit) | M0 |
| `OBU_METADATA` (ITU-T T.35, HDR CLL, HDR MDCV, scalability, timecode) | §5.8 | ☐ | M4 |
| `OBU_PADDING` / `OBU_REDUNDANT_FRAME_HEADER` | §5.7/§5.9 | ☐ | — |
| `OBU_TILE_LIST` (large-scale tiles; forbidden in AVIF item) | §5.12 | ☐ | — |
| seq_profile=1 (High) | Annex A §10.2; §6.4.1 | ✅ | M0 |
| seq_profile=0 (Main) / =2 (Professional, 12-bit/4:2:2) | Annex A §10.2 | ☐ | M2 |
| `still_picture`=1, `reduced_still_picture_header`=1 | §5.5 | ✅ | M0 |
| full seq header: timing_info, decoder_model_info, multiple operating points | §5.5.1-.5.5.5 | ☐ | M6 |
| `frame_id_numbers_present` | §5.5 | ☐ | — |
| `use_128x128_superblock` | §5.5 | ☐ | M1 |
| `enable_filter_intra` (1 on lossy, 0 on lossless) / `enable_intra_edge_filter`=0 | §5.5 | ✅ | M0/M1 |
| `enable_superres`/`cdef`/`restoration`=0 | §5.5 | ✅ (off) | M0 |
| color_config: mc=0 identity, 4:4:4, high_bitdepth=0, full range | §5.5.2 | ✅ | M0 |
| color_config: high_bitdepth/twelve_bit, mono_chrome, subsampling, chroma_sample_position | §5.5.2 | ☐ | M2 |
| frame_type=KEY_FRAME, show_frame=1 | §5.9.2 | ✅ | M0 |
| INTRA_ONLY / INTER / SWITCH frame types | §5.9.2 | ☐ | M6 |
| `disable_cdf_update`=1 (static CDFs) | §5.9.2 | ✅ | M0 |
| `disable_cdf_update`=0 + frame-end CDF update | §5.9.2,§7.7 | ☐ | M1 |
| frame_size / render_size (no override, no superres) | §5.9.5/.6 | ✅ | M0 |
| superres_params (enable_superres + use_superres + coded_denom) | §5.9.8,§7.16 | ✅ (frame_size_override deferred) | M1 |
| tile_info: single tile | §5.9.15 | ✅ | M0 |
| multi-tile (uniform spacing, tile_size_bytes, context_update_tile_id, tile group) | §5.9.15/.16 | ✅ (2 cols ≥2 SB wide; rows deferred) | M1 |
| quantization: base_q_idx=0 ⇒ CodedLossless | §5.9.12 | ✅ | M0 |
| quantization: base_q_idx>0, delta-Q, using_qmatrix | §5.9.12/.13,§9.5 | ✅ (base_q_idx>0 + per-SB delta-Q; qmatrix ☐) | M1 |
| segmentation_params (disabled) | §5.9.14 | ✅ (off) | M0 |
| segmentation (8 segments, features, temporal pred) | §5.9.14 | ✅ (lossy; SEG_LVL_ALT_Q + spatial segment_id map; temporal pred ☐) | M1 |
| delta_q_params / delta_lf_params | §5.9.17/.18 | ✅ (delta_q + delta_lf) | M1 |
| read_tx_mode → ONLY_4X4 (lossless) | §5.9.21 | ✅ | M0 |
| TX_MODE_SELECT / TX_MODE_LARGEST | §5.9.21 | ✅ (TX_MODE_SELECT, lossy intra) | M1 |
| `reduced_tx_set`=1 | §5.9.2 | ✅ | M0 |
| frame_reference_mode / skip_mode_params (intra → off) | §5.9.22/.23 | ✅ (off) | M0 |
| global_motion_params | §5.9.24 | ☐ | M6 |
| film_grain_params | §5.9.30 | ☐ | M4 |

## C. AV1 — tiling, partition, block / mode info

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| single-tile `decode_tile`, above/left context clear | §5.11.2/.3 | ✅ | M0 |
| `decode_partition`: PARTITION_NONE + edge-forced SPLIT/HORZ/VERT | §5.11.4 | ✅ | M0 |
| full partition set: HORZ/VERT/SPLIT/HORZ_A/B/VERT_A/B/HORZ_4/VERT_4 | §5.11.4 | ✅ (HORZ/VERT + NONE/SPLIT; A/B/4 deferred) | M1 |
| rectangular transforms TX_16X8/8X16/32X16/16X32 (+scan, aspect coeff ctx) | §7.13.3/§8.3.2 | ✅ | M1 |
| `intra_frame_mode_info` (KEY-frame block) | §5.11.7 | ✅ | M0 |
| `skip` flag = 0 (residual always coded) | §5.11.11 | ✅ | M0 |
| `skip` = 1 (no-residual / all-zero blocks) | §5.11.11 | ✅ (lossy; all-skip 8×8 unfiltered by CDEF) | M1 |
| intra_segment_id / read_segment_id (spatial pred, neg_interleave) | §5.11.8/.9 | ✅ (lossy multi-segment) | M0/M1 |
| per-block read_cdef / read_delta_qindex / read_delta_lf | §5.11.56/.12/.13 | ✅ (delta_q + delta_lf; read_cdef 0-bit) | M1 |
| read_tx_size / read_var_tx_size (per-block tx_depth) | §5.11.15-.17 | ✅ (TX_MODE_SELECT, square tx_depth 0..2) | M0/M1 |

## D. AV1 — intra prediction (§7.11.2)

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| `DC_PRED`, availability-aware (luma + chroma) | §7.11.2.5 | ✅ | M0 |
| directional V/H/D45/.../D67 + `angle_delta` + edge filter/upsample | §7.11.2.4/.9-.12 | ✅ (lossy luma 4×4; 8×8/16×16/32×32 + `angle_delta`; edge filter/upsample `enable_intra_edge_filter=0`) | M1 |
| SMOOTH / SMOOTH_V / SMOOTH_H | §7.11.2.6 | ☐ | M1 |
| PAETH | §7.11.2 | ☐ | M1 |
| recursive filter-intra | §7.11.2.3,§5.11.24 | ✅ (lossy luma 4×4 + 8×8 + 16×16 + 32×32) | M1 |
| chroma-from-luma (CfL) + `cfl_alpha` | §7.11.5,§5.11.45 | ✅ (lossy 4:4:4 4×4) | M1 |
| palette mode (palette_tokens, color cache) | §7.11.4,§5.11.46-.50 | ✅ (lossy luma 8×8/16×16/32×32; sizes 2..8; color cache + wavefront index map) | M1 |
| intra block copy (`allow_intrabc`) | §7.11.x,§5.11.x | ☐ | M1 |

## E. AV1 — transforms (§7.13)

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| inverse 4×4 Walsh-Hadamard (lossless) + matched forward | §7.13.2.10 | ✅ | M0 |
| inverse DCT 4/8/16/32/64 + forward DCT | §7.13.2.2/.3 | ✅ (4/8/16/32/64, used through TX_64X64) | M1 |
| inverse ADST4/8/16 (+FLIPADST) + forward | §7.13.2.4-.9 | ☐ | M1 |
| identity transform 4/8/16/32 (IDTX / V_ / H_) | §7.13.2.11-.15 | ☐ | M1 |
| 2D inverse transform + tx_type sets, `get_tx_set` | §7.13.3,§5.11.47/.48 | ☐ | M1 |
| variable tx size / `txfm_split` | §5.11.15-.17 | ☐ | M1 |
| encoder forward transform + tx-type/size RD search | (encoder) | ☐ | M1 |

## F. AV1 — quantization (§7.12)

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| lossless dequant (q_idx 0) feeding WHT reconstruct | §7.12.2/.3 | ✅ | M0 |
| dc_q/ac_q lookup tables (8/10/12-bit) | §7.12.2 | ☐ | M1/M2 |
| quantizer matrices (qm_y/u/v) | §9.5 | ☐ | M1 |
| encoder quantization (dead-zone, RDOQ) | (encoder) | ☐ | M1 |

## G. AV1 — entropy coding & tables (§8, §9)

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| symbol/range encoder (inverse of §8.2 decoder) | §8.2 | ✅ | M0 |
| `encode_literal` (equiprobable `read_bool` inverse) | §8.2.3/.5 | ✅ | M0 |
| static default CDFs: Partition, Skip, IntraFrameYMode, UvMode(±CfL) | §9.4 | ✅ | M0 |
| coeff CDFs (qctx0, TX_4X4): TxbSkip/EobPt16/EobExtra/CoeffBaseEob/CoeffBase/CoeffBr/DcSign | §9.4 | ✅ | M0 |
| full default CDF tables: all qctx, tx classes, inter/MV/palette | §9.4 | ☐ | M1/M6 |
| CDF adaptation + frame-end update + context_update_tile | §8.2.6,§7.7 | ☐ | M1 |
| `coeffs()` TX_4X4: txb_skip/eob/base/br/sign/dc_sign/golomb | §5.11.39 | ✅ | M0 |
| `coeffs()` all tx sizes + transform_type signaling | §5.11.39/.47 | ✅ (lossy 4×4 + 8×8 + 16×16 + 32×32 + 64×64, 32×32/64×64 DCT-only) | M1 |
| scan table `Default_Scan_4x4` + context-offset tables | §9.2/§9.3/§8.3.2 | ✅ | M0 |
| all scan tables (default/col/row per tx size) | §9.2 | ✅ (4×4 + 8×8 + 16×16 + 32×32 + 64×64 default) | M1 |

## H. AV1 — in-loop filters & post (§7.14-§7.18; all bypassed under CodedLossless)

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| deblocking loop filter | §5.9.11,§7.14 | ✅ (lossy 4×4/8×8/16×16, narrow + wide + widest) | M1 |
| CDEF (constrained directional enhancement filter) | §5.9.19,§7.15 | ✅ (lossy 4:4:4) | M1 |
| loop restoration: Wiener (luma) + stripe boundaries + per-SB unit signaling | §5.9.20,§7.17 | ✅ (Wiener luma; self-guided/chroma deferred) | M1 |
| superres horizontal upscaling (8-tap polyphase, LR after upscale) | §5.9.8,§7.16 | ✅ (opt-in via `encode_still_intra_superres`) | M1 |
| film grain synthesis | §5.9.30,§7.18.3 | ☐ | M4 |

## I. AV1 — inter coding (image sequences `avis` only; AVIF still image is intra-only)

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| reference frame buffers, ref_frame_idx, order hint | §5.9,§7.20/.21 | ☐ | M6 |
| MV prediction (find_mv_stack), MV/MVD coding | §7.10,§5.11.25-.34 | ☐ | M6 |
| inter prediction: single/compound, OBMC, warped, wedge, masked | §7.11.3 | ☐ | M6 |
| skip_mode, ref_frame_mvs, global motion, motion-field estimation | §5.9.22/.24,§7.9 | ☐ | M6 |

## J. Color / CICP / HDR / metadata

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| identity matrix (mc=0), full range, 4:4:4, planar G/B/R mapping | CICP H.273; §5.5.2 | ✅ | M0 |
| BT.601/709/2020 matrices (mc=1/5/6/9), limited range | CICP H.273 | ☐ | M2 |
| RGB↔YCbCr + chroma down/up-sample (4:2:0/4:2:2) | (gamut-color) | ☐ | M2 |
| transfer sRGB/BT.709 (tagged only in M0) | CICP H.273 | ✅ (tag) | M0 |
| transfer PQ (SMPTE ST 2084) / HLG (BT.2100) | CICP H.273 | ☐ | M4 |
| primaries variants; embedded ICC profile | CICP; 23008-12 | ☐ | M4 |
| HDR mastering display (`mdcv`) + content light level (`clli`) | §5.8.3/.4 | ☐ | M4 |

## K. Cross-crate API, I/O & tooling

| Component | Spec | Status | M |
| --- | --- | --- | --- |
| `gamut_core::Encoder` impl (RGB8 assumption) | gamut-core | ✅ | M0 |
| `AvifEncoder::encode_rgb8` native API | gamut-avif | ✅ | M0 |
| RGBA8 input + alpha-plane extraction | gamut-color/avif | ☐ | M3 |
| 10/12/16-bit & float HDR input buffers | gamut-color | ☐ | M2/M4 |
| quality/speed config + rate control | gamut-avif/av1 | ☐ | M1 |
| `gamut_core::Decoder` (AVIF → pixels) | gamut-avif | ☐ | future |
| CLI / wasm / ffi wiring for AVIF | gamut-{cli,wasm,ffi} | ☐ | future |
