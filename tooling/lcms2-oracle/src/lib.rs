//! Dev-only differential oracle around a vendored, statically-linked **Little-CMS (lcms2)**.
//!
//! `gamut-icc` must parse the ICC profiles a reference CMM writes and re-serialize profiles that
//! CMM accepts as equivalent. This crate wraps lcms2 (built from the `third_party/lcms2` submodule)
//! behind a small, safe API with two halves:
//!
//! * **synthesis** — build a diverse corpus of valid profiles *in memory*, so no binary `.icc`
//!   fixtures need committing: [`srgb`], [`rgb_matrix_shaper`], [`gray`], [`xyz`], [`lab4`],
//!   [`lab2`];
//! * **inspection** — open a profile blob and read back header fields and decoded tag values
//!   ([`Profile::from_bytes`], [`Profile::color_space`], [`Profile::read_xyz`], …).
//!
//! Profiles work entirely in RAM via `cmsSaveProfileToMem`/`cmsOpenProfileFromMem`, so — unlike the
//! file-based libtiff/DNG oracles — there is no temp-file round-trip. All `unsafe` FFI is confined
//! to this crate; returned values are copied out of lcms2's memory before the handle is closed.

#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case)]

use std::os::raw::c_char;
use std::ptr;

mod sys {
    // Generated bindings: vendored, machine-emitted code we do not lint.
    #![allow(warnings, clippy::all)]
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

/// ICC tag signatures (a four-character code as a big-endian `u32`) accepted by the `tag` argument
/// of the read-back methods. Mirrors the subset of `cmsTagSignature` the cross-checks exercise.
pub mod tag {
    const fn sig(b: &[u8; 4]) -> u32 {
        u32::from_be_bytes([b[0], b[1], b[2], b[3]])
    }
    /// `rXYZ` — red colorant (`XYZType`).
    pub const RED_COLORANT: u32 = sig(b"rXYZ");
    /// `gXYZ` — green colorant.
    pub const GREEN_COLORANT: u32 = sig(b"gXYZ");
    /// `bXYZ` — blue colorant.
    pub const BLUE_COLORANT: u32 = sig(b"bXYZ");
    /// `wtpt` — media white point.
    pub const MEDIA_WHITE_POINT: u32 = sig(b"wtpt");
    /// `rTRC` — red tone-response curve.
    pub const RED_TRC: u32 = sig(b"rTRC");
    /// `gTRC` — green tone-response curve.
    pub const GREEN_TRC: u32 = sig(b"gTRC");
    /// `bTRC` — blue tone-response curve.
    pub const BLUE_TRC: u32 = sig(b"bTRC");
    /// `kTRC` — grey tone-response curve.
    pub const GRAY_TRC: u32 = sig(b"kTRC");
    /// `desc` — profile description.
    pub const PROFILE_DESCRIPTION: u32 = sig(b"desc");
    /// `cprt` — copyright.
    pub const COPYRIGHT: u32 = sig(b"cprt");
    /// `chad` — chromatic-adaptation matrix.
    pub const CHROMATIC_ADAPTATION: u32 = sig(b"chad");
}

/// A four-character colour-space / class signature as a big-endian `u32`, for comparing against the
/// values returned by [`Profile::color_space`], [`Profile::pcs`], and [`Profile::device_class`].
#[must_use]
pub const fn fourcc(b: &[u8; 4]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// An owned lcms2 profile handle (`cmsHPROFILE`); closed on drop.
pub struct Profile {
    raw: sys::cmsHPROFILE,
}

impl Drop for Profile {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            // SAFETY: `raw` is a live handle from an lcms2 constructor, closed exactly once here.
            unsafe { sys::cmsCloseProfile(self.raw) };
        }
    }
}

fn wrap(raw: sys::cmsHPROFILE) -> Profile {
    assert!(!raw.is_null(), "lcms2 returned a null profile handle");
    Profile { raw }
}

/// An owned lcms2 tone curve, freed on drop. Used only to feed the profile constructors.
struct ToneCurve(*mut sys::cmsToneCurve);

impl ToneCurve {
    fn gamma(g: f64) -> Self {
        // SAFETY: global context (null) is valid; returns an owned curve (checked non-null below).
        let p = unsafe { sys::cmsBuildGamma(ptr::null_mut(), g) };
        assert!(!p.is_null(), "cmsBuildGamma returned null");
        Self(p)
    }
}

impl Drop for ToneCurve {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `0` is an owned curve from `cmsBuildGamma`, freed exactly once.
            unsafe { sys::cmsFreeToneCurve(self.0) };
        }
    }
}

const fn xyy(x: f64, y: f64, big_y: f64) -> sys::cmsCIExyY {
    sys::cmsCIExyY { x, y, Y: big_y }
}

// ---- Synthesis: build valid profiles in memory ------------------------------------------------

/// The built-in sRGB profile (`cmsCreate_sRGBProfile`): a v4 matrix/TRC display RGB profile.
#[must_use]
pub fn srgb() -> Profile {
    // SAFETY: constructor returns an owned handle (checked by `wrap`).
    wrap(unsafe { sys::cmsCreate_sRGBProfile() })
}

/// An RGB matrix/TRC profile from a white point `(x, y)`, primaries `(x, y)` per channel, and a
/// per-channel pure-gamma tone curve.
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

impl Profile {
    /// Force the encoded profile version (e.g. `2.1`, `4.3`), so synthesis can emit legacy v2
    /// layouts (`textDescriptionType`, v2 LUTs) for the cross-checks.
    pub fn set_version(&self, version: f64) {
        // SAFETY: `raw` is a live handle.
        unsafe { sys::cmsSetProfileVersion(self.raw, version) };
    }

    /// Serialize this profile to ICC bytes (`cmsSaveProfileToMem`, two-call size-then-fill).
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut needed: sys::cmsUInt32Number = 0;
        // SAFETY: a null buffer with a valid out-param requests the size.
        let ok = unsafe { sys::cmsSaveProfileToMem(self.raw, ptr::null_mut(), &mut needed) };
        assert!(
            ok != 0 && needed > 0,
            "cmsSaveProfileToMem size query failed"
        );
        let mut buf = vec![0u8; needed as usize];
        // SAFETY: `buf` has room for `needed` bytes; lcms writes exactly that many.
        let ok =
            unsafe { sys::cmsSaveProfileToMem(self.raw, buf.as_mut_ptr().cast(), &mut needed) };
        assert!(ok != 0, "cmsSaveProfileToMem write failed");
        buf.truncate(needed as usize);
        buf
    }

    /// Open an ICC byte blob with lcms2 (`cmsOpenProfileFromMem`). Returns `None` if lcms2 rejects
    /// the bytes — the basis of the round-trip gate ("does the reference CMM accept our output?").
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<Profile> {
        let len = sys::cmsUInt32Number::try_from(bytes.len()).ok()?;
        // SAFETY: `bytes` is valid for `len` bytes; lcms copies what it needs.
        let raw = unsafe { sys::cmsOpenProfileFromMem(bytes.as_ptr().cast(), len) };
        (!raw.is_null()).then_some(Profile { raw })
    }

    /// The profile version as a float (e.g. `4.3`), via `cmsGetProfileVersion`.
    #[must_use]
    pub fn version(&self) -> f64 {
        // SAFETY: `raw` is a live handle.
        unsafe { sys::cmsGetProfileVersion(self.raw) }
    }

    /// The device-class signature as a big-endian `u32` (e.g. `mntr`), via `cmsGetDeviceClass`.
    #[must_use]
    pub fn device_class(&self) -> u32 {
        // SAFETY: `raw` is a live handle.
        unsafe { sys::cmsGetDeviceClass(self.raw) as u32 }
    }

    /// The data colour-space signature as a big-endian `u32` (e.g. `RGB `), via `cmsGetColorSpace`.
    #[must_use]
    pub fn color_space(&self) -> u32 {
        // SAFETY: `raw` is a live handle.
        unsafe { sys::cmsGetColorSpace(self.raw) as u32 }
    }

    /// The profile-connection-space signature as a big-endian `u32` (`XYZ ` or `Lab `), via
    /// `cmsGetPCS`.
    #[must_use]
    pub fn pcs(&self) -> u32 {
        // SAFETY: `raw` is a live handle.
        unsafe { sys::cmsGetPCS(self.raw) as u32 }
    }

    /// The header's default rendering intent (0–3), via `cmsGetHeaderRenderingIntent`.
    #[must_use]
    pub fn rendering_intent(&self) -> u32 {
        // SAFETY: `raw` is a live handle.
        unsafe { sys::cmsGetHeaderRenderingIntent(self.raw) }
    }

    /// The header flags word, via `cmsGetHeaderFlags`.
    #[must_use]
    pub fn header_flags(&self) -> u32 {
        // SAFETY: `raw` is a live handle.
        unsafe { sys::cmsGetHeaderFlags(self.raw) }
    }

    /// The 16-byte profile ID currently stored in the header, via `cmsGetHeaderProfileID`.
    #[must_use]
    pub fn profile_id(&self) -> [u8; 16] {
        let mut id = [0u8; 16];
        // SAFETY: `raw` is live; lcms writes exactly 16 bytes into `id`.
        unsafe { sys::cmsGetHeaderProfileID(self.raw, id.as_mut_ptr()) };
        id
    }

    /// Recompute the profile ID (MD5) per the spec and return it (`cmsMD5computeID` then read back).
    #[must_use]
    pub fn compute_md5_id(&self) -> [u8; 16] {
        // SAFETY: `raw` is a live handle.
        unsafe { sys::cmsMD5computeID(self.raw) };
        self.profile_id()
    }

    /// Read an `XYZType` tag as `[X, Y, Z]`, via `cmsReadTag`. `None` if the tag is absent.
    #[must_use]
    pub fn read_xyz(&self, tag: u32) -> Option<[f64; 3]> {
        // SAFETY: `raw` is live; the returned pointer is borrowed and copied out immediately.
        let p = unsafe { sys::cmsReadTag(self.raw, tag as sys::cmsTagSignature) }
            as *const sys::cmsCIEXYZ;
        if p.is_null() {
            return None;
        }
        // SAFETY: non-null pointer to a live `cmsCIEXYZ` owned by the profile.
        let v = unsafe { *p };
        Some([v.X, v.Y, v.Z])
    }

    /// Evaluate a tone-curve tag at `x ∈ [0, 1]` (`cmsEvalToneCurveFloat`). `None` if absent.
    #[must_use]
    pub fn eval_tone_curve(&self, tag: u32, x: f32) -> Option<f32> {
        // SAFETY: `raw` is live; the curve pointer is borrowed for the call only.
        let c = unsafe { sys::cmsReadTag(self.raw, tag as sys::cmsTagSignature) }
            as *const sys::cmsToneCurve;
        if c.is_null() {
            return None;
        }
        // SAFETY: non-null borrowed curve owned by the profile.
        Some(unsafe { sys::cmsEvalToneCurveFloat(c, x) })
    }

    /// Estimate the gamma of a tone-curve tag (`cmsEstimateGamma`). `None` if absent or non-power.
    #[must_use]
    pub fn estimate_gamma(&self, tag: u32, precision: f64) -> Option<f64> {
        // SAFETY: `raw` is live; the curve pointer is borrowed for the call only.
        let c = unsafe { sys::cmsReadTag(self.raw, tag as sys::cmsTagSignature) }
            as *const sys::cmsToneCurve;
        if c.is_null() {
            return None;
        }
        // SAFETY: non-null borrowed curve owned by the profile.
        let g = unsafe { sys::cmsEstimateGamma(c, precision) };
        (g > 0.0).then_some(g)
    }

    /// Read a `multiLocalizedUnicodeType`/`textDescriptionType` tag as an ASCII string for the
    /// given language/country (e.g. `b"en"`, `b"US"`), via `cmsMLUgetASCII`. `None` if absent.
    #[must_use]
    pub fn read_mlu_ascii(&self, tag: u32, lang: &[u8; 2], country: &[u8; 2]) -> Option<String> {
        // lcms language/country codes are 2 letters in a 3-byte (NUL-padded) field.
        let lang = [lang[0] as c_char, lang[1] as c_char, 0];
        let country = [country[0] as c_char, country[1] as c_char, 0];
        // SAFETY: `raw` is live; the MLU pointer is borrowed and only read during this call.
        let m =
            unsafe { sys::cmsReadTag(self.raw, tag as sys::cmsTagSignature) } as *const sys::cmsMLU;
        if m.is_null() {
            return None;
        }
        // SAFETY: size query — a null buffer returns the byte count needed.
        let need =
            unsafe { sys::cmsMLUgetASCII(m, lang.as_ptr(), country.as_ptr(), ptr::null_mut(), 0) };
        if need == 0 {
            return None;
        }
        let mut buf = vec![0u8; need as usize];
        // SAFETY: `buf` holds `need` bytes; lcms writes a NUL-terminated ASCII string into it.
        unsafe {
            sys::cmsMLUgetASCII(
                m,
                lang.as_ptr(),
                country.as_ptr(),
                buf.as_mut_ptr().cast(),
                need,
            );
        }
        if buf.last() == Some(&0) {
            buf.pop();
        }
        Some(String::from_utf8_lossy(&buf).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_round_trips_through_lcms() {
        let p = srgb();
        let bytes = p.to_bytes();
        assert!(bytes.len() > 128, "serialized profile too small");
        // 'acsp' magic at offset 36.
        assert_eq!(&bytes[36..40], b"acsp");
        let reopened = Profile::from_bytes(&bytes).expect("lcms2 re-opens its own output");
        assert_eq!(reopened.color_space(), fourcc(b"RGB "));
        assert_eq!(reopened.pcs(), fourcc(b"XYZ "));
        // The white point colorant is present and close to D50.
        let wtpt = reopened
            .read_xyz(tag::MEDIA_WHITE_POINT)
            .expect("wtpt present");
        assert!(
            (wtpt[1] - 1.0).abs() < 1e-3,
            "wtpt Y ≈ 1.0, got {}",
            wtpt[1]
        );
    }

    #[test]
    fn rejects_non_icc_bytes() {
        assert!(Profile::from_bytes(b"not an icc profile").is_none());
    }
}
