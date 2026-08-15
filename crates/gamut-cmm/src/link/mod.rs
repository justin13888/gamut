//! Profile linking: builds runnable [`Pipeline`]s from parsed ICC profiles.
//!
//! [`device_to_pcs`] and [`pcs_to_device`] turn one [`gamut_icc::IccProfile`] into one half of
//! a colour conversion, over the crate's sample convention (device channels encoded `[0, 1]`,
//! PCS seams **decoded** colorimetry — XYZ with D50 `Y = 1.0`, Lab with `L*` in `0..=100`).
//! Linking a profile *pair* is composing the two halves
//! ([`Pipeline::compose`](crate::Pipeline::compose)).
//!
//! # Phase coverage (and the lcms2 dispatch rule)
//!
//! Little-CMS resolves a profile to a pipeline by trying the intent's LUT tag first
//! (`A2Bx`/`B2Ax`, with fallbacks) and falling back to the matrix/TRC ("shaper") tags only
//! when no LUT tag exists (`_cmsReadInputLUT`/`_cmsReadOutputLUT`, `cmsio1.c`). This module
//! keeps that precedence: a profile carrying LUT tags for the requested direction is refused
//! with [`CmmError::UnsupportedProfile`] **until issue #328 lands the LUT path** — the shaper
//! tags such a profile may also carry are never silently used in its place. What builds today
//! is the shaper set: RGB and gray matrix/TRC profiles (v2 and v4), XYZ PCS, plus the gray
//! Lab-PCS form; see [`CmmError::UnsupportedProfile`] for every refusal boundary.
//!
//! # Chromatic-adaptation convention (the v2/v4 `chad` decision)
//!
//! Colorant tags (`rXYZ`/`gXYZ`/`bXYZ`) are consumed **as-is**, for v2 and v4 profiles alike:
//! ICC.1:2022 §8.3.4 requires them to be already D50-adapted, and the `chad` tag is **never
//! read** on this relative-colorimetric path — matching lcms2, whose only `chad` consumer is
//! the absolute-intent white-point scaling (`cmscnvrt.c`), itself inert at the default
//! adaptation state. A strict reading of some v2 profiles (colorants relative to the actual
//! white, `chad` meant to adapt them) would disagree; this crate deliberately follows lcms2.
//! The `wtpt` tag is likewise reserved to absolute intent (#329). The full audit and the
//! differential tests pinning it live in `STATUS.md` ("Settled decisions (P4)") and
//! `tests/oracle_shaper.rs`.

mod shaper;

use gamut_icc::{ColorSpace, IccProfile, KnownTag, RenderingIntent};

use crate::error::{CmmError, Result};
use crate::pipeline::Pipeline;

/// The device→PCS LUT tags, presence of any of which routes a profile to the (future) LUT
/// path — the union of lcms2's per-intent `Device2PCS16` table (`cmsio1.c`).
const DEVICE_TO_PCS_LUTS: [KnownTag; 3] = [KnownTag::AToB0, KnownTag::AToB1, KnownTag::AToB2];

/// The PCS→device LUT tags (lcms2's `PCS2Device16` table).
const PCS_TO_DEVICE_LUTS: [KnownTag; 3] = [KnownTag::BToA0, KnownTag::BToA1, KnownTag::BToA2];

/// Whether any of the direction's LUT tags is present.
fn has_lut_tags(profile: &IccProfile, tags: &[KnownTag; 3]) -> bool {
    tags.iter().any(|&tag| profile.get(tag).is_some())
}

/// Rejects a PCS this phase cannot decode into: the shaper builders produce decoded XYZ or
/// (gray only) decoded Lab, so the header must claim one of the two ICC connection spaces.
fn check_pcs(profile: &IccProfile) -> Result<()> {
    match profile.header.pcs {
        ColorSpace::Xyz | ColorSpace::Lab => Ok(()),
        _ => Err(CmmError::UnsupportedProfile(
            "shaper linking requires an XYZ or Lab PCS",
        )),
    }
}

/// Builds the device → decoded-PCS half of a conversion for `profile` — the
/// relative-colorimetric baseline.
///
/// The pipeline consumes one pixel of encoded `[0, 1]` device channels and produces decoded
/// PCS colorimetry (XYZ with D50 `Y = 1.0`, or Lab with `L*` in `0..=100` for a gray Lab-PCS
/// profile). `intent` is accepted for API stability but does not alter the result at this
/// phase: a shaper profile has no per-intent tables (perceptual/saturation renderings live in
/// LUT tags, #328), and ICC-absolute colorimetric — the one intent that changes shaper
/// output, via the `wtpt` white scaling — arrives with issue #329.
///
/// # Errors
///
/// [`CmmError::UnsupportedProfile`] if the profile carries a device→PCS LUT tag
/// (`A2B0`/`A2B1`/`A2B2` — the LUT path arrives with issue #328), its PCS is neither XYZ nor
/// Lab, its device space is neither RGB nor gray, or it is an RGB shaper with a Lab PCS
/// (arrives with #328's `XyzToLab` stage); [`CmmError::MissingTag`] /
/// [`CmmError::BadTagType`] for absent or unusable colorant/TRC tags; and any
/// [`ToneCurve::new`](crate::ToneCurve::new) error for a malformed TRC.
pub fn device_to_pcs(profile: &IccProfile, intent: RenderingIntent) -> Result<Pipeline> {
    // Intent-invariant at this phase (see above); the parameter exists so callers do not
    // change signature when #328/#329 make it observable.
    let _ = intent;
    if has_lut_tags(profile, &DEVICE_TO_PCS_LUTS) {
        return Err(CmmError::UnsupportedProfile(
            "LUT-tag pipelines arrive with issue #328",
        ));
    }
    check_pcs(profile)?;
    match profile.header.data_color_space {
        ColorSpace::Rgb => shaper::rgb_device_to_pcs(profile),
        ColorSpace::Gray => shaper::gray_device_to_pcs(profile),
        _ => Err(CmmError::UnsupportedProfile(
            "only matrix/TRC shaper profiles are supported at this phase",
        )),
    }
}

/// Builds the decoded-PCS → device half of a conversion for `profile` — the
/// relative-colorimetric baseline.
///
/// The mirror of [`device_to_pcs`]: consumes decoded PCS colorimetry, produces encoded
/// `[0, 1]` device channels. `intent` is accepted but inert at this phase (see
/// [`device_to_pcs`]).
///
/// # Errors
///
/// As [`device_to_pcs`] (with `B2A0`/`B2A1`/`B2A2` as the refused LUT tags), plus
/// [`CmmError::SingularMatrix`] if the colorant matrix has no finite inverse and
/// [`CmmError::NonMonotonicCurve`] if a TRC has no functional inverse.
pub fn pcs_to_device(profile: &IccProfile, intent: RenderingIntent) -> Result<Pipeline> {
    let _ = intent;
    if has_lut_tags(profile, &PCS_TO_DEVICE_LUTS) {
        return Err(CmmError::UnsupportedProfile(
            "LUT-tag pipelines arrive with issue #328",
        ));
    }
    check_pcs(profile)?;
    match profile.header.data_color_space {
        ColorSpace::Rgb => shaper::rgb_pcs_to_device(profile),
        ColorSpace::Gray => shaper::gray_pcs_to_device(profile),
        _ => Err(CmmError::UnsupportedProfile(
            "only matrix/TRC shaper profiles are supported at this phase",
        )),
    }
}

#[cfg(test)]
mod tests {
    use gamut_icc::{
        ColorSpace, Curve, DeviceClass, IccProfile, ProfileHeader, Signature, TagData,
    };

    use super::*;

    /// A shaper-less profile skeleton over the given spaces (no tags at all).
    fn bare_profile(device: ColorSpace, pcs: ColorSpace) -> IccProfile {
        let mut header = ProfileHeader::new(DeviceClass::Display, device);
        header.pcs = pcs;
        IccProfile {
            header,
            tags: Vec::new(),
        }
    }

    #[test]
    fn lut_tags_take_precedence_and_are_refused_per_direction() {
        // Presence of any LUT tag for the requested direction refuses the profile even though
        // the payload is never inspected (precedence is a tag-presence rule, as in lcms2).
        let lut = TagData::Raw {
            type_sig: Signature(*b"mft2"),
            bytes: Vec::new(),
        };
        for a2b in [*b"A2B0", *b"A2B1", *b"A2B2"] {
            let mut profile = bare_profile(ColorSpace::Rgb, ColorSpace::Xyz);
            profile.tags.push((Signature(a2b), lut.clone()));
            let err =
                device_to_pcs(&profile, RenderingIntent::MediaRelativeColorimetric).unwrap_err();
            assert_eq!(
                err.to_string(),
                "cmm: unsupported profile (LUT-tag pipelines arrive with issue #328)"
            );
            // The opposite direction ignores A2B tags (and then trips on the missing shaper
            // tags instead — a different, non-LUT error).
            let err =
                pcs_to_device(&profile, RenderingIntent::MediaRelativeColorimetric).unwrap_err();
            assert!(matches!(err, CmmError::MissingTag(_)), "got {err}");
        }
        for b2a in [*b"B2A0", *b"B2A1", *b"B2A2"] {
            let mut profile = bare_profile(ColorSpace::Rgb, ColorSpace::Xyz);
            profile.tags.push((Signature(b2a), lut.clone()));
            let err =
                pcs_to_device(&profile, RenderingIntent::MediaRelativeColorimetric).unwrap_err();
            assert_eq!(
                err.to_string(),
                "cmm: unsupported profile (LUT-tag pipelines arrive with issue #328)"
            );
            let err =
                device_to_pcs(&profile, RenderingIntent::MediaRelativeColorimetric).unwrap_err();
            assert!(matches!(err, CmmError::MissingTag(_)), "got {err}");
        }
    }

    #[test]
    fn non_rgb_gray_device_spaces_are_refused() {
        for device in [ColorSpace::Cmyk, ColorSpace::Xyz, ColorSpace::Lab] {
            let profile = bare_profile(device, ColorSpace::Xyz);
            for build in [device_to_pcs, pcs_to_device] {
                let err = build(&profile, RenderingIntent::MediaRelativeColorimetric).unwrap_err();
                assert_eq!(
                    err.to_string(),
                    "cmm: unsupported profile (only matrix/TRC shaper profiles are supported at \
                     this phase)"
                );
            }
        }
    }

    #[test]
    fn non_connection_space_pcs_is_refused() {
        // A device-link-style header (device space in the PCS field) cannot be a shaper seam.
        let profile = bare_profile(ColorSpace::Rgb, ColorSpace::Rgb);
        for build in [device_to_pcs, pcs_to_device] {
            let err = build(&profile, RenderingIntent::MediaRelativeColorimetric).unwrap_err();
            assert_eq!(
                err.to_string(),
                "cmm: unsupported profile (shaper linking requires an XYZ or Lab PCS)"
            );
        }
    }

    #[test]
    fn intent_does_not_alter_the_built_pipeline_at_this_phase() {
        // A gray shaper built under each of the four intents evaluates identically — the
        // documented invariance (perceptual/saturation need LUT tags, absolute arrives with
        // #329).
        let mut profile = bare_profile(ColorSpace::Gray, ColorSpace::Xyz);
        profile.tags.push((
            Signature(*b"kTRC"),
            TagData::Curve(Curve::Gamma(gamut_icc::U8Fixed8(0x0233))),
        ));
        let baseline = device_to_pcs(&profile, RenderingIntent::MediaRelativeColorimetric).unwrap();
        for intent in [
            RenderingIntent::Perceptual,
            RenderingIntent::Saturation,
            RenderingIntent::IccAbsoluteColorimetric,
        ] {
            let other = device_to_pcs(&profile, intent).unwrap();
            for g in [0.0, 0.25, 0.5, 1.0] {
                let (mut a, mut b) = ([0.0; 3], [0.0; 3]);
                baseline.eval(&[g], &mut a).unwrap();
                other.eval(&[g], &mut b).unwrap();
                assert_eq!(a, b, "intent {intent:?} diverged at {g}");
            }
        }
    }
}
