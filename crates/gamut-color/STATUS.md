# gamut-color — color primitives implementation status

The shared color layer of the [gamut](../../README.md) codec campaign: CICP (H.273 /
ISO/IEC 23091-2) code points, coded-plane metadata (AV1 §5.5.2/§6.4.2), identity and BT.601
YCbCr 4:2:0 planar buffers (RFC 6386 §9.2 / libwebp), and the Tier-1 `f64` colour science
documented in [`references/color`](../../references/color/README.md). Released as **v1**
(issue #179).

**Keystone (done):** the **derivation-vs-literal `M1` verification** plus the **encoder-exact
transfer set** — every per-gamut OKLab `M1` table is re-derived from CIE chromaticities via
Lindbloom's RGB→XYZ construction and Bradford adaptation and must match the hand-transcribed
literals to `1e-7` (`derived_matrices_match_literals`), so no colour constant is trusted as a
copy-paste; and the transfer layer exposes both the encoder-exact curves (Adobe `x^2.2`,
ProPhoto `x^1.8`, BT.2020 PQ→Reinhard@203) and the standard curves, pinned to differ.

**Oracle:** published external values, no FFI — Bruce Lindbloom's sRGB→XYZ (D65) and Bradford
D65→D50 matrices; Ottosson's published OKLab `M1`/`M2` constants; an independent BT.2100
forward-OETF inversion of `pq_eotf`; chromahash golden vectors (`unit-color.json`,
`unit-softgamutclamp.json`, MIT OR Apache-2.0); and libwebp's integer-exact `VP8RGBToY/U/V`
anchors plus the JFIF/BT.601 full-range anchors. Determinism is **Tier-1** (correctness only,
`std` `f64`): golden-vector agreement is tolerance-level, not bit-for-bit.

## Phases

| Phase | Scope | Status |
| ----- | ----- | ------ |
| P1 | Scaffold + CICP/format tables: H.273 code-point enums, `BitDepth`, `ChromaSubsampling` | ✅ |
| P2 | `Planar8` identity (`mc = 0`, GBR) 4:4:4 buffers for the AV1/AVIF path | ✅ |
| P3 | BT.601 YCbCr 4:2:0 (`Yuv420`) with libwebp-exact limited-range integer math (WebP/VP8) | ✅ |
| P4 | Colour science: `transfer`, `oklab`, `matrix` (Bradford), `gamut_map`, `profile` | ✅ |
| P5 | **Keystone** — `M1` derivation cross-check + encoder-exact vs standard curves | ✅ |
| P6 | Tone-map dedupe with `gamut-tonemap`; luminance levels sourced from `gamut_core::luminance` | ✅ |
| v1 | Issue #179 — API finalization (`non_exhaustive` policy, profile surface, code-point inverses), overflow-safe constructors, AV1 §6.4.2 monochrome fix, oracle-only minimal test set | ✅ |
| P7 | `xyb` (issue #334): the JPEG XL opsin space — frozen libjxl 0.12.0 constants (absorbance matrix + bias + normative inverse, transcribed in `references/color/README.md` with `references/jxl/opsin_params.h` vendored), `linear_srgb_to_xyb`/`xyb_to_linear_srgb`, and the scaled-XYB byte encoding (`scale_xyb`/`unscale_xyb`, third stored channel `B − Y`) for `gamut-jpeg`'s XYB colour mode. Tier-1 f64 like the rest of the colour science | ✅ |

## API policies frozen at v1

- **Per-source-spec naming.** H.273 identifiers keep the spec's British spelling
  (`ColourPrimaries`); AV1-derived names use American spelling (`ColorRange`, `clip_pixel`).
- **Growth-bound enums are `#[non_exhaustive]`** (CICP code points, `BitDepth`,
  `ChromaSubsampling`, `Gamut`, `SourceTransfer`); `ColorRange` is deliberately exhaustive (a
  spec-complete single bit). New variants and `from_code_point` `None`→`Some` promotions are
  minor releases.
- **`gamut_core::Error` is the error surface** — the crate's only fallible paths are
  buffer-shape validation, exactly `InvalidInput`-shaped; no crate-local error enum.
- **Range is a conversion property, not buffer state**: `Yuv420` does not store the
  `ColorRange` used to fill it; callers carry the flag end to end (it is header metadata).
- **`oklab_to_linear_srgb` is sRGB-only by design** — sRGB is the gamut-membership test target
  and the encode target; per-gamut inverses are additive later if a use case appears.

## Intentionally deferred (additive)

- **10/12-bit encode wiring** — the *presentation* direction now covers 8/10/12/16-bit
  (`YcbcrMatrix`); only the encode side is deferred. `BitDepth::Ten`/`Twelve` are modeled; the AV1 reconstruction
  accepts them but no encode path produces them yet (gamut-avif M2).
- **16-bit is modeled, not an AV1 depth** — `BitDepth::Sixteen` (issue #260) is not deferred AV1
  wiring like `Ten`/`Twelve`: AV1 tops out at 12 and never produces it. It exists for the 16-bit
  interleaved still-image pipelines (PNG/TIFF/JXL/DNG) that consume this vocabulary. `clip_pixel`
  accepts it; no gamut encode path emits it. Depths outside the fixed 8/10/12/16 set (RAW's
  14-bit, TIFF's `1..=16`) are deliberately **not** enum variants — those formats carry a
  free-form integer depth (`gamut-dng`'s `bits_per_sample`, validated `1..=16`), a domain an enum
  cannot model.
- **Subsampled coded planes** — `Cs422`/`Cs420`/`Cs400` wiring into AV1 (M2); today 4:2:0
  exists only in the WebP/VP8 `Yuv420` path.
- **`MatrixCoefficients::YCgCo`** — modeled, with no de-matrixing yet: it is a lifting transform,
  not a `Kr`/`Kb` matrix. Lands with M4. `Identity` is deliberately *not* a `YcbcrMatrix` either —
  it is a GBR plane permutation the caller performs, not an affine transform — and `Unspecified` is
  a policy choice belonging to the format layer.
- **Row / SIMD YCbCr→RGB** — `YcbcrMatrix::to_rgb` is scalar and `#[inline]`; a subsampling-aware
  row API is additive if profiling asks for it.
- **RGB → YCbCr at generic depth and matrix** — `YcbcrMatrix` implements the inverse only; the
  forward direction is additive.
- **HLG and BT.709 transfer curves** — `eotf_for` returns `None` for them (M4 HDR).
- **Better chroma resampling** (sharp-YUV-style) — the box filter is the documented baseline.
- **Bit-reproducible math substrate** — chromahash's `cbrt_halley`/portable-pow tier is
  deliberately not ported (Tier-1 policy, issue #37).
