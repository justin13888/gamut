//! Profile synthesis: builds a diverse corpus of valid ICC profiles *in memory* (no binary
//! `.icc` fixtures), from lcms2's virtual-profile constructors and from hand-assembled
//! pipeline/CLUT tags.
//!
//! LUT-writing order matters: `cmsWriteTag` resolves the serialized tag *type* from the
//! profile's **current** version (`DecideLUTtypeA2B`/`B2A` in `cmstypes.c`), so every builder
//! here calls `cmsSetProfileVersion` **before** writing A2B/B2A tags — `< 4.0` serializes
//! `lut16`, `>= 4.0` serializes `mAB `/`mBA `.

use std::os::raw::c_void;
use std::ptr;

use crate::curves::ToneCurve;
use crate::{Profile, fourcc, sys, tag, wrap};

pub(crate) const fn xyy(x: f64, y: f64, big_y: f64) -> sys::cmsCIExyY {
    sys::cmsCIExyY { x, y, Y: big_y }
}

/// The built-in sRGB profile (`cmsCreate_sRGBProfile`): a v4 matrix/TRC display RGB profile.
#[must_use]
pub fn srgb() -> Profile {
    // SAFETY: constructor returns an owned handle (checked by `wrap`).
    wrap(unsafe { sys::cmsCreate_sRGBProfile() })
}

/// An RGB matrix/TRC profile from a white point `(x, y)`, primaries `(x, y)` per channel, and a
/// per-channel pure-gamma tone curve.
///
/// Per `cmsCreateRGBProfileTHR`, the `wtpt` tag is written as **D50** regardless of `white`; the
/// requested white lands in the `chad` tag instead (and the colorants are Bradford-adapted to
/// D50). See [`rgb_matrix_shaper_d65_wtpt`] for the non-D50-`wtpt` variant.
#[must_use]
pub fn rgb_matrix_shaper(white: [f64; 2], primaries: [[f64; 2]; 3], gamma: [f64; 3]) -> Profile {
    let wp = xyy(white[0], white[1], 1.0);
    let prim = sys::cmsCIExyYTRIPLE {
        Red: xyy(primaries[0][0], primaries[0][1], 1.0),
        Green: xyy(primaries[1][0], primaries[1][1], 1.0),
        Blue: xyy(primaries[2][0], primaries[2][1], 1.0),
    };
    let curves = [
        ToneCurve::gamma(gamma[0]),
        ToneCurve::gamma(gamma[1]),
        ToneCurve::gamma(gamma[2]),
    ];
    let mut raw = [curves[0].0, curves[1].0, curves[2].0];
    // SAFETY: all pointers outlive the call; lcms copies the curve data into the new profile.
    wrap(unsafe { sys::cmsCreateRGBProfile(&wp, &prim, raw.as_mut_ptr()) })
}

/// A grey-scale profile from a white point `(x, y)` and a pure-gamma tone curve.
///
/// `cmsCreateGrayProfile` writes the curve as the `kTRC` (grey TRC) tag and — unlike the RGB
/// constructor — stores the **actual** white under `wtpt`, with no `chad`; this is already the
/// grey/kTRC vehicle, no separate synthesizer is needed.
#[must_use]
pub fn gray(white: [f64; 2], gamma: f64) -> Profile {
    let wp = xyy(white[0], white[1], 1.0);
    let tc = ToneCurve::gamma(gamma);
    // SAFETY: pointers outlive the call; lcms copies the curve into the profile.
    wrap(unsafe { sys::cmsCreateGrayProfile(&wp, tc.0) })
}

/// The built-in CIE XYZ identity profile (`cmsCreateXYZProfile`).
#[must_use]
pub fn xyz() -> Profile {
    // SAFETY: constructor returns an owned handle.
    wrap(unsafe { sys::cmsCreateXYZProfile() })
}

/// A v4 CIE L\*a\*b\* profile with the D50 white point (`cmsCreateLab4Profile(NULL)`).
#[must_use]
pub fn lab4() -> Profile {
    // SAFETY: NULL white point selects D50; returns an owned handle.
    wrap(unsafe { sys::cmsCreateLab4Profile(ptr::null()) })
}

/// A v2 CIE L\*a\*b\* profile with the D50 white point (`cmsCreateLab2Profile(NULL)`).
#[must_use]
pub fn lab2() -> Profile {
    // SAFETY: NULL white point selects D50; returns an owned handle.
    wrap(unsafe { sys::cmsCreateLab2Profile(ptr::null()) })
}

/// A CMYK ink-limiting device link (`cmsCreateInkLimitingDeviceLink`): a CLUT-bearing LUT profile,
/// for exercising the `lut8`/`lut16` decoders (force v2 with [`Profile::set_version`]).
#[must_use]
pub fn cmyk_ink_limiting_devicelink(limit: f64) -> Profile {
    let colorspace = u32::from_be_bytes(*b"CMYK") as sys::cmsColorSpaceSignature;
    // SAFETY: constructor returns an owned handle.
    wrap(unsafe { sys::cmsCreateInkLimitingDeviceLink(colorspace, limit) })
}

/// An RGB linearization device link (`cmsCreateLinearizationDeviceLink`) with identity curves, for
/// exercising the `lutAToB` decoder in v4 profiles.
#[must_use]
pub fn rgb_linearization_devicelink() -> Profile {
    let curves = [
        ToneCurve::gamma(1.0),
        ToneCurve::gamma(1.0),
        ToneCurve::gamma(1.0),
    ];
    let mut raw = [curves[0].0, curves[1].0, curves[2].0];
    let colorspace = u32::from_be_bytes(*b"RGB ") as sys::cmsColorSpaceSignature;
    // SAFETY: the curve pointers outlive the call; lcms copies them into the profile.
    wrap(unsafe { sys::cmsCreateLinearizationDeviceLink(colorspace, raw.as_mut_ptr()) })
}

/// A blank RGB→XYZ display profile placeholder, ready for a single tag to be written and the whole
/// serialized — the base for the [`measurement`]/[`viewing_conditions`]/[`cicp`] cross-checks. The
/// class/space fields are set so `gamut-icc` (which validates them) accepts the header.
fn placeholder() -> sys::cmsHPROFILE {
    placeholder_with(4.3, b"mntr", b"RGB ", b"XYZ ")
}

/// A blank placeholder with explicit version/class/space/PCS. The version is set here — before
/// any `cmsWriteTag` — because LUT tag types are resolved from it at write time (module docs).
fn placeholder_with(
    version: f64,
    class: &[u8; 4],
    space: &[u8; 4],
    pcs: &[u8; 4],
) -> sys::cmsHPROFILE {
    // SAFETY: null context selects the global context; returns an owned handle (asserted non-null).
    let raw = unsafe { sys::cmsCreateProfilePlaceholder(ptr::null_mut()) };
    assert!(!raw.is_null(), "cmsCreateProfilePlaceholder returned null");
    // SAFETY: `raw` is a live handle; each setter takes a signature as its enum-typed u32.
    unsafe {
        sys::cmsSetProfileVersion(raw, version);
        sys::cmsSetDeviceClass(raw, fourcc(class) as sys::cmsProfileClassSignature);
        sys::cmsSetColorSpace(raw, fourcc(space) as sys::cmsColorSpaceSignature);
        sys::cmsSetPCS(raw, fourcc(pcs) as sys::cmsColorSpaceSignature);
    }
    raw
}

/// Writes `data` under tag `code` and returns the owned profile (`cmsWriteTag`).
fn with_tag(raw: sys::cmsHPROFILE, code: &[u8; 4], data: *const c_void) -> Profile {
    // SAFETY: `raw` is live; `data` points at a valid struct of the type lcms expects for `code`,
    // which lcms serializes immediately during the call.
    let ok = unsafe { sys::cmsWriteTag(raw, fourcc(code) as sys::cmsTagSignature, data) };
    assert!(
        ok != 0,
        "cmsWriteTag failed for {:?}",
        core::str::from_utf8(code)
    );
    wrap(raw)
}

/// A profile carrying one `measurementType` tag (`meas`) with the given fields, for cross-checking
/// the `gamut-icc` decoder against lcms2's own serialization.
#[must_use]
pub fn measurement(
    observer: u32,
    backing: [f64; 3],
    geometry: u32,
    flare: f64,
    illuminant: u32,
) -> Profile {
    let cond = sys::cmsICCMeasurementConditions {
        Observer: observer,
        Backing: sys::cmsCIEXYZ {
            X: backing[0],
            Y: backing[1],
            Z: backing[2],
        },
        Geometry: geometry,
        Flare: flare,
        IlluminantType: illuminant,
    };
    with_tag(
        placeholder(),
        b"meas",
        (&cond as *const sys::cmsICCMeasurementConditions).cast(),
    )
}

/// A profile carrying one `viewingConditionsType` tag (`view`) with the given un-normalized CIEXYZ
/// illuminant/surround and illuminant type.
#[must_use]
pub fn viewing_conditions(
    illuminant: [f64; 3],
    surround: [f64; 3],
    illuminant_type: u32,
) -> Profile {
    let cond = sys::cmsICCViewingConditions {
        IlluminantXYZ: sys::cmsCIEXYZ {
            X: illuminant[0],
            Y: illuminant[1],
            Z: illuminant[2],
        },
        SurroundXYZ: sys::cmsCIEXYZ {
            X: surround[0],
            Y: surround[1],
            Z: surround[2],
        },
        IlluminantType: illuminant_type,
    };
    with_tag(
        placeholder(),
        b"view",
        (&cond as *const sys::cmsICCViewingConditions).cast(),
    )
}

/// A profile carrying one `cicpType` tag (`cicp`) with the given four H.273 code points.
#[must_use]
pub fn cicp(primaries: u8, transfer: u8, matrix: u8, full_range: u8) -> Profile {
    let signal = sys::cmsVideoSignalType {
        ColourPrimaries: primaries,
        TransferCharacteristics: transfer,
        MatrixCoefficients: matrix,
        VideoFullRangeFlag: full_range,
    };
    with_tag(
        placeholder(),
        b"cicp",
        (&signal as *const sys::cmsVideoSignalType).cast(),
    )
}

// ---- LUT/CLUT synthesis -----------------------------------------------------------------------

/// Clamp to the unit interval (the CLUT samplers work in normalized channel space).
fn clamp01(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}

/// `u16` code → normalized `[0, 1]` channel value.
fn norm(v: u16) -> f64 {
    f64::from(v) / 65535.0
}

/// Normalized `[0, 1]` channel value → `u16` code, round-half-up like lcms2's samplers.
fn denorm(v: f64) -> u16 {
    (clamp01(v) * 65535.0 + 0.5) as u16
}

/// Reads the warp parameter the sampler cargo points at.
///
/// # Safety
///
/// `cargo` must point at a live `f64` (guaranteed by [`write_lut_tag`], the only sampler
/// installer in this crate).
unsafe fn warp_of(cargo: *mut c_void) -> f64 {
    // SAFETY: per the function contract, `cargo` is the `&mut f64` passed to
    // `cmsStageSampleCLut16bit` by `write_lut_tag`, live for the whole sampling loop.
    unsafe { *cargo.cast::<f64>() }
}

/// CLUT sampler: a smooth deterministic RGB→Lab warp (device RGB in, v4-encoded 16-bit PCSLAB
/// out) for the [`scnr_lut`] profile. Not colorimetrically meaningful — just smooth, in-range,
/// and channel-mixing so interpolation errors are visible.
unsafe extern "C" fn sample_rgb_to_lab(
    input: *const sys::cmsUInt16Number,
    output: *mut sys::cmsUInt16Number,
    _cargo: *mut c_void,
) -> sys::cmsInt32Number {
    // SAFETY: lcms passes 3 input and 3 output entries for a 3→3 CLUT.
    let (inp, out) = unsafe {
        (
            std::slice::from_raw_parts(input, 3),
            std::slice::from_raw_parts_mut(output, 3),
        )
    };
    let (r, g, b) = (norm(inp[0]), norm(inp[1]), norm(inp[2]));
    let l = 0.15 + 0.8 * (0.35 * r + 0.5 * g + 0.15 * b).powf(1.1);
    let a = 0.5 + 0.18 * (r - g) + 0.04 * (b - 0.5);
    let bb = 0.5 + 0.18 * (g - b) + 0.04 * (r - 0.5);
    out[0] = denorm(l);
    out[1] = denorm(a);
    out[2] = denorm(bb);
    1
}

/// CLUT sampler: a smooth deterministic CMYK→Lab warp for the `prtr` A2B pipelines. The cargo's
/// warp parameter (0/1/2 for the perceptual/colorimetric/saturation tags) bends lightness and
/// shifts chroma so per-intent outputs measurably differ.
unsafe extern "C" fn sample_cmyk_to_lab(
    input: *const sys::cmsUInt16Number,
    output: *mut sys::cmsUInt16Number,
    cargo: *mut c_void,
) -> sys::cmsInt32Number {
    // SAFETY: lcms passes 4 input and 3 output entries for a 4→3 CLUT; cargo is `write_lut_tag`'s
    // warp f64.
    let (inp, out, w) = unsafe {
        (
            std::slice::from_raw_parts(input, 4),
            std::slice::from_raw_parts_mut(output, 3),
            warp_of(cargo),
        )
    };
    let (c, m, y, k) = (norm(inp[0]), norm(inp[1]), norm(inp[2]), norm(inp[3]));
    let ink = 0.2 * c + 0.25 * m + 0.2 * y + 0.35 * k;
    let l = clamp01(1.0 - ink).powf(1.0 + 0.15 * w);
    let a = 0.5 + (0.22 * (m - c) + 0.03 * w * (y - 0.5)) * (1.0 - 0.6 * k);
    let b = 0.5 + (0.22 * (y - m) - 0.03 * w * (c - 0.5)) * (1.0 - 0.6 * k);
    out[0] = denorm(l);
    out[1] = denorm(a);
    out[2] = denorm(b);
    1
}

/// CLUT sampler: a smooth deterministic Lab→CMYK warp for the `prtr` B2A pipelines (not the
/// inverse of [`sample_cmyk_to_lab`] — output profiles need no such consistency to parse or to
/// drive transforms).
unsafe extern "C" fn sample_lab_to_cmyk(
    input: *const sys::cmsUInt16Number,
    output: *mut sys::cmsUInt16Number,
    cargo: *mut c_void,
) -> sys::cmsInt32Number {
    // SAFETY: lcms passes 3 input and 4 output entries for a 3→4 CLUT; cargo is `write_lut_tag`'s
    // warp f64.
    let (inp, out, w) = unsafe {
        (
            std::slice::from_raw_parts(input, 3),
            std::slice::from_raw_parts_mut(output, 4),
            warp_of(cargo),
        )
    };
    let (l, a, b) = (norm(inp[0]), norm(inp[1]), norm(inp[2]));
    let dark = 1.0 - l;
    out[0] = denorm(0.6 * dark - 0.35 * (a - 0.5) + 0.05 * w * (b - 0.5));
    out[1] = denorm(0.6 * dark + 0.35 * (a - 0.5) - 0.05 * w * (l - 0.5));
    out[2] = denorm(0.6 * dark + 0.35 * (b - 0.5) + 0.05 * w * (a - 0.5));
    out[3] = denorm(dark.powf(1.4 + 0.1 * w));
    1
}

/// Builds `identity curves → 16-bit CLUT(grid) → identity curves` and writes it under `sig`.
///
/// Identity curves come from `cmsStageAllocToneCurves(n, NULL)` — the exported spelling of the
/// internal `_cmsStageAllocIdentityCurves`, which bindgen does not emit. The CLUT is filled by
/// `sampler` via `cmsStageSampleCLut16bit`, with `warp` passed through the cargo pointer. The
/// pipeline is freed after the write (`cmsWriteTag` stores a duplicate).
fn write_lut_tag(
    raw: sys::cmsHPROFILE,
    sig: u32,
    grid: u32,
    in_ch: u32,
    out_ch: u32,
    sampler: sys::cmsSAMPLER16,
    warp: f64,
) {
    let mut warp = warp;
    // SAFETY: every stage is checked non-null and ownership moves into the pipeline on insert;
    // the sampler contract (channel counts, live cargo) is documented on each sampler; the
    // pipeline outlives the write and is freed exactly once.
    unsafe {
        let lut = sys::cmsPipelineAlloc(ptr::null_mut(), in_ch, out_ch);
        assert!(!lut.is_null(), "cmsPipelineAlloc failed");
        let pre = sys::cmsStageAllocToneCurves(ptr::null_mut(), in_ch, ptr::null());
        assert!(!pre.is_null(), "identity input curves failed");
        assert!(sys::cmsPipelineInsertStage(lut, sys::cmsAT_END, pre) != 0);
        let clut = sys::cmsStageAllocCLut16bit(ptr::null_mut(), grid, in_ch, out_ch, ptr::null());
        assert!(!clut.is_null(), "cmsStageAllocCLut16bit failed");
        assert!(
            sys::cmsStageSampleCLut16bit(clut, sampler, (&mut warp as *mut f64).cast(), 0) != 0,
            "cmsStageSampleCLut16bit failed"
        );
        assert!(sys::cmsPipelineInsertStage(lut, sys::cmsAT_END, clut) != 0);
        let post = sys::cmsStageAllocToneCurves(ptr::null_mut(), out_ch, ptr::null());
        assert!(!post.is_null(), "identity output curves failed");
        assert!(sys::cmsPipelineInsertStage(lut, sys::cmsAT_END, post) != 0);
        assert!(
            sys::cmsWriteTag(raw, sig as sys::cmsTagSignature, lut.cast_const().cast()) != 0,
            "cmsWriteTag failed for LUT tag {sig:#010x}"
        );
        sys::cmsPipelineFree(lut);
    }
}

/// Maps a CLUT probe channel count onto a device colour-space signature: the shapes the CLUT
/// differential tests need (1 → `GRAY`, 3 → `RGB `, 4 → `CMYK`).
fn probe_space(channels: u32) -> &'static [u8; 4] {
    match channels {
        1 => b"GRAY",
        3 => b"RGB ",
        4 => b"CMYK",
        n => panic!("clut probe profiles support 1/3/4 channels, got {n}"),
    }
}

/// A **devicelink-class** profile wrapping an arbitrary caller-supplied 16-bit CLUT: `A2B0` is
/// identity curves → `cmsStageAllocCLut16bitGranular(grid_points, samples)` → identity curves,
/// written at version 4.3 (serializes as `mAB `). Colour space and "PCS" (a link profile's
/// output space) are chosen by channel count ([`probe_space`]); drive it with
/// [`Transform::devicelink`](crate::xform::Transform::devicelink).
///
/// `samples` are in grid order (last input axis fastest, output channels interleaved per
/// node), `prod(grid_points) × out_ch` entries; lcms2 copies them.
///
/// Two behavioural notes for differential tests:
/// - `PreOptimize` runs even under `FLAGS_NOOPTIMIZE`, but only strips
///   `cmsSigIdentityElemType` stages and folds matrices; the identity *tone-curve* stages here
///   (`cmsStageAllocToneCurves(n, NULL)` = gamma-1.0 curves) survive, so the pipeline reaching
///   `cmsDoTransform` is exactly curves → CLUT → curves (verified empirically by the
///   node-reproduction test below).
/// - A profile-borne 16-bit CLUT is evaluated through lcms2's **fixed-point** interpolators
///   even in a double-precision transform (`EvaluateCLUTfloatIn16`), so agreement is
///   16-bit-tight only; the float-path oracle is [`ClutPipeline`](crate::xform::ClutPipeline).
#[must_use]
pub fn clut_probe_profile(grid_points: &[u8], samples: &[u16], out_ch: u32) -> Profile {
    let in_ch = u32::try_from(grid_points.len()).expect("axis count fits u32");
    let nodes: usize = grid_points.iter().map(|&n| usize::from(n)).product();
    assert_eq!(
        samples.len(),
        nodes * out_ch as usize,
        "sample count must be prod(grid) x out_ch"
    );
    let raw = placeholder_with(4.3, b"link", probe_space(in_ch), probe_space(out_ch));
    let points: Vec<sys::cmsUInt32Number> = grid_points.iter().map(|&n| u32::from(n)).collect();
    // SAFETY: every stage is checked non-null and ownership moves into the pipeline on insert;
    // lcms2 copies `points`/`samples` during the allocation call; the pipeline is freed exactly
    // once after `cmsWriteTag` stores a duplicate.
    unsafe {
        let lut = sys::cmsPipelineAlloc(ptr::null_mut(), in_ch, out_ch);
        assert!(!lut.is_null(), "cmsPipelineAlloc failed");
        let pre = sys::cmsStageAllocToneCurves(ptr::null_mut(), in_ch, ptr::null());
        assert!(!pre.is_null(), "identity input curves failed");
        assert!(sys::cmsPipelineInsertStage(lut, sys::cmsAT_END, pre) != 0);
        let clut = sys::cmsStageAllocCLut16bitGranular(
            ptr::null_mut(),
            points.as_ptr(),
            in_ch,
            out_ch,
            samples.as_ptr(),
        );
        assert!(!clut.is_null(), "cmsStageAllocCLut16bitGranular failed");
        assert!(sys::cmsPipelineInsertStage(lut, sys::cmsAT_END, clut) != 0);
        let post = sys::cmsStageAllocToneCurves(ptr::null_mut(), out_ch, ptr::null());
        assert!(!post.is_null(), "identity output curves failed");
        assert!(sys::cmsPipelineInsertStage(lut, sys::cmsAT_END, post) != 0);
        assert!(
            sys::cmsWriteTag(
                raw,
                tag::A_TO_B0 as sys::cmsTagSignature,
                lut.cast_const().cast()
            ) != 0,
            "cmsWriteTag failed for A2B0"
        );
        sys::cmsPipelineFree(lut);
    }
    wrap(raw)
}

/// Overwrites the `wtpt` tag with the XYZ of the chromaticity `white` (via `cmsxyY2XYZ`).
fn write_wtpt(raw: sys::cmsHPROFILE, white: [f64; 2]) {
    let wp = xyy(white[0], white[1], 1.0);
    let mut wxyz = sys::cmsCIEXYZ {
        X: 0.0,
        Y: 0.0,
        Z: 0.0,
    };
    // SAFETY: both pointers are live locals; lcms copies the XYZ into the tag store.
    unsafe {
        sys::cmsxyY2XYZ(&mut wxyz, &wp);
        assert!(
            sys::cmsWriteTag(
                raw,
                tag::MEDIA_WHITE_POINT as sys::cmsTagSignature,
                (&wxyz as *const sys::cmsCIEXYZ).cast(),
            ) != 0,
            "cmsWriteTag(wtpt) failed"
        );
    }
}

/// An RGB matrix/TRC profile retagged as an **Input** (`scnr`) class device — scanner-shaped
/// metadata over the same colorant/TRC tags (`cmsSetDeviceClass` is a pure header edit).
#[must_use]
pub fn scnr_matrix_shaper(white: [f64; 2], primaries: [[f64; 2]; 3], gamma: [f64; 3]) -> Profile {
    let p = rgb_matrix_shaper(white, primaries, gamma);
    // SAFETY: `raw` is a live handle; the setter only edits the header field.
    unsafe { sys::cmsSetDeviceClass(p.raw, fourcc(b"scnr") as sys::cmsProfileClassSignature) };
    p
}

/// An Input (`scnr`) class RGB→Lab **v4 LUT profile**: `A2B0` is identity curves → a smooth
/// deterministic CLUT warp ([`sample_rgb_to_lab`]) → identity curves, written at version 4.3 so
/// it serializes as `mAB `.
#[must_use]
pub fn scnr_lut(grid: u32) -> Profile {
    let raw = placeholder_with(4.3, b"scnr", b"RGB ", b"Lab ");
    write_lut_tag(raw, tag::A_TO_B0, grid, 3, 3, Some(sample_rgb_to_lab), 0.0);
    wrap(raw)
}

/// The D65 white chromaticity, used as the deliberately-non-D50 `wtpt` of the `prtr` profiles
/// (so the absolute-colorimetric white scaling `diag(whiteIn/whiteOut)` is not the identity and
/// absolute-intent differentials are non-vacuous).
const D65_XY: [f64; 2] = [0.3127, 0.3290];

/// Shared body of [`cmyk_prtr_v4`]/[`cmyk_prtr_v2`]: an Output (`prtr`) CMYK↔Lab profile at the
/// given version, carrying all **six** LUT tags — `A2B0/1/2` (4→3) and `B2A0/1/2` (3→4) — each
/// CLUT warped differently per intent slot, plus a D65 `wtpt`.
fn cmyk_prtr(version: f64, grid: u32) -> Profile {
    let raw = placeholder_with(version, b"prtr", b"CMYK", b"Lab ");
    for (slot, (a2b, b2a)) in [
        (tag::A_TO_B0, tag::B_TO_A0),
        (tag::A_TO_B1, tag::B_TO_A1),
        (tag::A_TO_B2, tag::B_TO_A2),
    ]
    .into_iter()
    .enumerate()
    {
        let warp = slot as f64;
        write_lut_tag(raw, a2b, grid, 4, 3, Some(sample_cmyk_to_lab), warp);
        write_lut_tag(raw, b2a, grid, 3, 4, Some(sample_lab_to_cmyk), warp);
    }
    write_wtpt(raw, D65_XY);
    wrap(raw)
}

/// An Output (`prtr`) CMYK↔Lab **v4.4** profile with six per-intent LUT tags (see
/// [`cmyk_prtr`]); serializes them as `mAB `/`mBA `. A modest `grid` (e.g. 9) keeps it small.
#[must_use]
pub fn cmyk_prtr_v4(grid: u32) -> Profile {
    cmyk_prtr(4.4, grid)
}

/// The same six-tag Output CMYK↔Lab profile at **version 2.4** — set before the writes, so every
/// LUT serializes as `lut16` (`mft2`): the v2-Lab-encoding vehicle.
#[must_use]
pub fn cmyk_prtr_v2(grid: u32) -> Profile {
    cmyk_prtr(2.4, grid)
}

/// A Display P3 profile — P3 primaries, D65 white — with the **true sRGB piecewise TRC** as a
/// parametric type-4 curve (`[2.4, 1/1.055, 0.055/1.055, 1/12.92, 0.04045]`), shared across the
/// three channels (lcms links `gTRC`/`bTRC` to `rTRC` for pointer-identical curves).
#[must_use]
pub fn display_p3_srgb_trc() -> Profile {
    let wp = xyy(D65_XY[0], D65_XY[1], 1.0);
    let prim = sys::cmsCIExyYTRIPLE {
        Red: xyy(0.680, 0.320, 1.0),
        Green: xyy(0.265, 0.690, 1.0),
        Blue: xyy(0.150, 0.060, 1.0),
    };
    let trc = ToneCurve::parametric(4, &[2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045]);
    let mut raw = [trc.0, trc.0, trc.0];
    // SAFETY: all pointers outlive the call; lcms copies the curve data into the new profile.
    wrap(unsafe { sys::cmsCreateRGBProfile(&wp, &prim, raw.as_mut_ptr()) })
}

/// An RGB matrix/TRC profile forced down to **version 2.1** after construction; with
/// `with_chad == false` the `chad` tag is then deleted (`cmsWriteTag(sig, NULL)` — lcms2's
/// documented tag-deletion path in `cmsio0.c`), yielding the bare-colorant v2 shaper whose
/// white point lcms2 forcibly treats as D50 on read.
#[must_use]
pub fn rgb_matrix_shaper_v2(
    with_chad: bool,
    white: [f64; 2],
    primaries: [[f64; 2]; 3],
    gamma: [f64; 3],
) -> Profile {
    let p = rgb_matrix_shaper(white, primaries, gamma);
    p.set_version(2.1);
    if !with_chad {
        // SAFETY: `raw` is live; a NULL data pointer deletes the (present) tag.
        let ok = unsafe {
            sys::cmsWriteTag(
                p.raw,
                tag::CHROMATIC_ADAPTATION as sys::cmsTagSignature,
                ptr::null(),
            )
        };
        assert!(ok != 0, "cmsWriteTag(chad, NULL) did not delete the tag");
    }
    p
}

/// An RGB matrix/TRC **v4** profile whose `wtpt` tag is overwritten with the *actual* white's
/// XYZ — `cmsCreateRGBProfileTHR` writes `wtpt = D50` + `chad` by default, which makes
/// absolute-intent differentials vacuous; this is the non-D50-`wtpt` vehicle (issue #329).
#[must_use]
pub fn rgb_matrix_shaper_d65_wtpt(
    white: [f64; 2],
    primaries: [[f64; 2]; 3],
    gamma: [f64; 3],
) -> Profile {
    let p = rgb_matrix_shaper(white, primaries, gamma);
    write_wtpt(p.raw, white);
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xform::{
        FLAGS_NOCACHE, FLAGS_NOOPTIMIZE, INTENT_ABSOLUTE_COLORIMETRIC, INTENT_PERCEPTUAL,
        INTENT_RELATIVE_COLORIMETRIC, INTENT_SATURATION, TYPE_CMYK_DBL, TYPE_Lab_DBL, Transform,
        set_quiet_log_handler,
    };

    /// The serialized element-type fourcc of tag `sig`, read straight out of the ICC byte layout
    /// (tag table at 128, 12-byte entries, type at each tag's data offset).
    fn serialized_tag_type(bytes: &[u8], sig: u32) -> Option<[u8; 4]> {
        let count = u32::from_be_bytes(bytes[128..132].try_into().unwrap()) as usize;
        (0..count).find_map(|i| {
            let e = 132 + 12 * i;
            let entry_sig = u32::from_be_bytes(bytes[e..e + 4].try_into().unwrap());
            (entry_sig == sig).then(|| {
                let off = u32::from_be_bytes(bytes[e + 4..e + 8].try_into().unwrap()) as usize;
                bytes[off..off + 4].try_into().unwrap()
            })
        })
    }

    const PRTR_LUT_TAGS: [u32; 6] = [
        tag::A_TO_B0,
        tag::A_TO_B1,
        tag::A_TO_B2,
        tag::B_TO_A0,
        tag::B_TO_A1,
        tag::B_TO_A2,
    ];

    /// The v4 `prtr` carries all six per-intent LUT tags as `mAB `/`mBA `, and one CMYK input
    /// transformed under each of the four `INTENT_*` values gives pairwise-different Lab —
    /// perceptual/relative/saturation via the per-tag warp, absolute vs relative via the D65
    /// `wtpt` white scaling.
    #[test]
    fn cmyk_prtr_v4_six_tags_and_distinct_intents() {
        set_quiet_log_handler();
        let prtr = cmyk_prtr_v4(9);
        let bytes = prtr.to_bytes();
        for sig in PRTR_LUT_TAGS {
            assert!(prtr.has_tag(sig), "missing LUT tag {sig:#010x}");
            let ty = serialized_tag_type(&bytes, sig).expect("tag in table");
            let want: &[u8; 4] = if sig >> 24 == u32::from(b'A') {
                b"mAB "
            } else {
                b"mBA "
            };
            assert_eq!(&ty, want, "tag {sig:#010x} serialized type");
        }

        let lab = lab4();
        let cmyk = [20.0, 45.0, 70.0, 10.0]; // ink percentages (TYPE_CMYK_DBL convention)
        let intents = [
            INTENT_PERCEPTUAL,
            INTENT_RELATIVE_COLORIMETRIC,
            INTENT_SATURATION,
            INTENT_ABSOLUTE_COLORIMETRIC,
        ];
        let outs: Vec<[f64; 3]> = intents
            .iter()
            .map(|&intent| {
                let t = Transform::new(
                    &prtr,
                    TYPE_CMYK_DBL,
                    &lab,
                    TYPE_Lab_DBL,
                    intent,
                    FLAGS_NOCACHE | FLAGS_NOOPTIMIZE,
                );
                let v = t.apply_f64(&cmyk, 1, 3);
                [v[0], v[1], v[2]]
            })
            .collect();
        for i in 0..outs.len() {
            for j in i + 1..outs.len() {
                let de = crate::color::delta_e_76(outs[i], outs[j]);
                assert!(
                    de > 0.05,
                    "intents {} and {} coincide: {:?} vs {:?}",
                    intents[i],
                    intents[j],
                    outs[i],
                    outs[j]
                );
            }
        }
    }

    /// The v2 `prtr` serializes every LUT tag as `lut16` (`mft2`) — the version was set before
    /// the writes, which is what `DecideLUTtypeA2B/B2A` keys on.
    #[test]
    fn cmyk_prtr_v2_serializes_lut16() {
        let prtr = cmyk_prtr_v2(9);
        let bytes = prtr.to_bytes();
        for sig in PRTR_LUT_TAGS {
            let ty = serialized_tag_type(&bytes, sig).expect("tag in table");
            assert_eq!(&ty, b"mft2", "tag {sig:#010x} serialized type");
        }
        assert!(prtr.version() < 4.0, "v2 profile version");
    }

    /// The `scnr` builders really produce Input-class profiles, and the LUT variant's A2B0
    /// serializes as `mAB ` and drives an RGB→Lab transform to plausible Lab values.
    #[test]
    fn scnr_profiles_are_input_class() {
        set_quiet_log_handler();
        let shaper = scnr_matrix_shaper(
            [0.3127, 0.3290],
            [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06]],
            [2.2, 2.2, 2.2],
        );
        assert_eq!(shaper.device_class(), fourcc(b"scnr"));

        let lut = scnr_lut(9);
        assert_eq!(lut.device_class(), fourcc(b"scnr"));
        assert_eq!(lut.pcs(), fourcc(b"Lab "));
        let bytes = lut.to_bytes();
        let ty = serialized_tag_type(&bytes, tag::A_TO_B0).expect("A2B0 present");
        assert_eq!(&ty, b"mAB ");

        let lab = lab4();
        let t = Transform::new(
            &lut,
            crate::xform::TYPE_RGB_DBL,
            &lab,
            TYPE_Lab_DBL,
            INTENT_RELATIVE_COLORIMETRIC,
            FLAGS_NOCACHE,
        );
        let out = t.apply_f64(&[0.8, 0.4, 0.2], 1, 3);
        assert!(out[0] > 0.0 && out[0] < 100.0, "L in range: {}", out[0]);
        assert!(out[1].abs() < 128.0 && out[2].abs() < 128.0);
    }

    /// The v2 shaper keeps/drops the `chad` tag as requested, and the d65-wtpt variant's `wtpt`
    /// reads back as D65 (not the D50 that `cmsCreateRGBProfileTHR` writes by default).
    #[test]
    fn shaper_variants_chad_and_wtpt() {
        let d65 = [0.3127, 0.3290];
        let prim = [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06]];
        let with = rgb_matrix_shaper_v2(true, d65, prim, [2.2, 2.2, 2.2]);
        assert!(with.has_tag(tag::CHROMATIC_ADAPTATION));
        assert!((with.version() - 2.1).abs() < 0.05);
        let without = rgb_matrix_shaper_v2(false, d65, prim, [2.2, 2.2, 2.2]);
        assert!(!without.has_tag(tag::CHROMATIC_ADAPTATION));
        // The deletion survives serialization.
        let reopened = Profile::from_bytes(&without.to_bytes()).expect("lcms reopens");
        assert!(!reopened.has_tag(tag::CHROMATIC_ADAPTATION));

        let d65_wtpt = rgb_matrix_shaper_d65_wtpt(d65, prim, [2.2, 2.2, 2.2]);
        let wtpt = d65_wtpt.read_xyz(tag::MEDIA_WHITE_POINT).expect("wtpt");
        assert!((wtpt[0] - 0.9505).abs() < 1e-3, "wtpt X = {}", wtpt[0]);
        assert!((wtpt[2] - 1.0891).abs() < 1e-3, "wtpt Z = {}", wtpt[2]);
        // The default builder writes D50 instead — the quirk this synthesizer works around.
        let plain = rgb_matrix_shaper(d65, prim, [2.2, 2.2, 2.2]);
        let wtpt = plain.read_xyz(tag::MEDIA_WHITE_POINT).expect("wtpt");
        assert!(
            (wtpt[0] - 0.9642).abs() < 1e-3,
            "default wtpt X = {}",
            wtpt[0]
        );
    }

    /// The empirical guard the CLUT probe's docs promise: the identity tone-curve stages are
    /// not collapsed by `PreOptimize` under `NOOPTIMIZE`, so exact node coordinates driven
    /// through `cmsDoTransform` reproduce the stored node samples (to 16-bit quantization).
    #[test]
    fn clut_probe_profile_reproduces_nodes_through_do_transform() {
        use crate::xform::{FLAGS_NOCACHE, FLAGS_NOOPTIMIZE, INTENT_PERCEPTUAL, TYPE_RGB_DBL};
        set_quiet_log_handler();
        // 3×3×3 RGB→RGB grid with distinct, well-spread node values.
        let samples: Vec<u16> = (0..27 * 3)
            .map(|i| (i * 811) % 65536)
            .map(|v| v as u16)
            .collect();
        let probe = clut_probe_profile(&[3, 3, 3], &samples, 3);
        assert_eq!(probe.device_class(), fourcc(b"link"));
        let t = crate::xform::Transform::devicelink(
            &probe,
            TYPE_RGB_DBL,
            TYPE_RGB_DBL,
            INTENT_PERCEPTUAL,
            FLAGS_NOCACHE | FLAGS_NOOPTIMIZE,
        );
        for (xi, x) in [0.0, 0.5, 1.0].into_iter().enumerate() {
            for (yi, y) in [0.0, 0.5, 1.0].into_iter().enumerate() {
                for (zi, z) in [0.0, 0.5, 1.0].into_iter().enumerate() {
                    let node = (xi * 3 + yi) * 3 + zi;
                    let out = t.apply_f64(&[x, y, z], 1, 3);
                    for ch in 0..3 {
                        let want = f64::from(samples[node * 3 + ch]) / 65535.0;
                        assert!(
                            (out[ch] - want).abs() < 1e-3,
                            "node {node} ch {ch}: {} vs {want}",
                            out[ch]
                        );
                    }
                }
            }
        }
    }

    /// `gray()` writes the curve under `kTRC` (documented on the synthesizer).
    #[test]
    fn gray_writes_ktrc() {
        let g = gray([0.3127, 0.3290], 2.2);
        assert!(g.has_tag(tag::GRAY_TRC));
        let mid = g.eval_tone_curve(tag::GRAY_TRC, 0.5).expect("kTRC");
        assert!((f64::from(mid) - 0.5f64.powf(2.2)).abs() < 1e-3);
    }
}
