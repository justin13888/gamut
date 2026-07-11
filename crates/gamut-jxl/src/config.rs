//! Encoder configuration: the [`Effort`] speed/density dial, the validated lossy [`Distance`]
//! newtype, the output [`Container`] selector, and the internal [`Mode`] that makes a
//! lossless-with-a-distance state unrepresentable.

use gamut_core::{Error, Result};

/// Encoder effort: the speed/density trade-off, from fastest ([`Effort::Lightning`]) to densest
/// ([`Effort::Glacier`]).
///
/// Maps directly onto libjxl's `JXL_ENC_FRAME_SETTING_EFFORT` levels `1..=10`; higher effort spends
/// more time searching for a smaller file at the same quality. The named variants are libjxl's own
/// codenames. The default is [`Effort::Squirrel`] (level 7), matching libjxl's default.
///
/// libjxl's level 11 ("tectonic plate") is expert-gated behind `JxlEncoderAllowExpertOptions` and is
/// deliberately out of scope, so this enum caps at level 10.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Effort {
    /// Level 1 — the fastest, lowest-density setting.
    Lightning,
    /// Level 2.
    Thunder,
    /// Level 3.
    Falcon,
    /// Level 4.
    Cheetah,
    /// Level 5.
    Hare,
    /// Level 6.
    Wombat,
    /// Level 7 — libjxl's default, a balanced speed/density point.
    #[default]
    Squirrel,
    /// Level 8.
    Kitten,
    /// Level 9.
    Tortoise,
    /// Level 10 — the slowest, highest-density setting.
    Glacier,
}

impl Effort {
    /// The libjxl effort level (`1..=10`) this variant selects.
    #[must_use]
    pub fn level(self) -> u8 {
        match self {
            Self::Lightning => 1,
            Self::Thunder => 2,
            Self::Falcon => 3,
            Self::Cheetah => 4,
            Self::Hare => 5,
            Self::Wombat => 6,
            Self::Squirrel => 7,
            Self::Kitten => 8,
            Self::Tortoise => 9,
            Self::Glacier => 10,
        }
    }

    /// The [`Effort`] for a libjxl effort level, or `None` if `level` is outside `1..=10`.
    ///
    /// The inverse of [`Effort::level`]; handy for wiring up a numeric CLI flag.
    #[must_use]
    pub fn from_level(level: u8) -> Option<Self> {
        Some(match level {
            1 => Self::Lightning,
            2 => Self::Thunder,
            3 => Self::Falcon,
            4 => Self::Cheetah,
            5 => Self::Hare,
            6 => Self::Wombat,
            7 => Self::Squirrel,
            8 => Self::Kitten,
            9 => Self::Tortoise,
            10 => Self::Glacier,
            _ => return None,
        })
    }
}

/// A validated Butteraugli target distance for lossy encoding.
///
/// The distance is libjxl's perceptual quality dial: `1.0` is "visually lossless" (the default) and
/// larger values trade visual quality for a smaller file, up to the maximum of `25.0`. A [`Distance`]
/// is guaranteed finite and in the half-open range `(0.0, 25.0]`.
///
/// `0.0` — libjxl's "mathematically lossless" sentinel — is deliberately **rejected**: lossless is a
/// distinct mode ([`crate::JxlEncoder::lossless`]), not a point on the lossy quality scale, and
/// keeping the two apart means a lossy configuration can never silently become lossless (or the
/// reverse) through a stray value.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Distance(f32);

impl Distance {
    /// Creates a [`Distance`], rejecting non-finite values and anything outside `(0.0, 25.0]`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `distance` is NaN or infinite, is `0.0` or negative, or
    /// exceeds `25.0`.
    pub fn new(distance: f32) -> Result<Self> {
        if distance.is_finite() && distance > 0.0 && distance <= 25.0 {
            Ok(Self(distance))
        } else {
            Err(Error::InvalidInput(
                "JXL: distance must be finite and in (0, 25]",
            ))
        }
    }

    /// The wrapped distance value.
    #[must_use]
    pub fn get(self) -> f32 {
        self.0
    }
}

impl Default for Distance {
    /// The "visually lossless" default distance of `1.0`.
    fn default() -> Self {
        Self(1.0)
    }
}

/// Which JPEG XL byte stream the encoder emits.
///
/// The two share the same coded image but differ in framing:
///
/// - [`Container::Codestream`] — a bare JPEG XL codestream, starting with the 2-byte signature
///   `0xFF 0x0A`. The smallest option; carries no room for metadata boxes.
/// - [`Container::IsoBmff`] — the ISO BMFF container (`.jxl` file format), starting with the 12-byte
///   box signature `00 00 00 0C 4A 58 4C 20 0D 0A 87 0A`. The framing needed to attach Exif/XMP/JUMBF
///   metadata boxes (metadata embedding itself is future work).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Container {
    /// A bare codestream (signature `0xFF 0x0A`); the default.
    #[default]
    Codestream,
    /// The ISO BMFF `.jxl` container (12-byte box signature).
    IsoBmff,
}

/// The encoder's lossless-vs-lossy state.
///
/// Kept private so that the invalid combination — lossless *and* a distance — is unrepresentable:
/// [`Mode::Lossless`] carries no distance, and [`Mode::Lossy`] always carries a validated one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Mode {
    /// Mathematically lossless: the decoded image is bit-exact to the input.
    Lossless,
    /// Lossy at the given Butteraugli [`Distance`].
    Lossy(Distance),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effort_level_round_trips_over_full_range() {
        // Every variant maps to a distinct 1..=10 level and back.
        let all = [
            Effort::Lightning,
            Effort::Thunder,
            Effort::Falcon,
            Effort::Cheetah,
            Effort::Hare,
            Effort::Wombat,
            Effort::Squirrel,
            Effort::Kitten,
            Effort::Tortoise,
            Effort::Glacier,
        ];
        for (i, e) in all.into_iter().enumerate() {
            let level = (i + 1) as u8;
            assert_eq!(e.level(), level, "{e:?} level");
            assert_eq!(Effort::from_level(level), Some(e), "from_level({level})");
        }
        // Exhaustive 1..=10 coverage: no gaps, no aliasing.
        for level in 1..=10u8 {
            assert_eq!(Effort::from_level(level).unwrap().level(), level);
        }
    }

    #[test]
    fn effort_from_level_rejects_out_of_range() {
        assert_eq!(Effort::from_level(0), None);
        assert_eq!(Effort::from_level(11), None);
        assert_eq!(Effort::from_level(255), None);
    }

    #[test]
    fn effort_default_is_squirrel() {
        assert_eq!(Effort::default(), Effort::Squirrel);
        assert_eq!(Effort::default().level(), 7);
    }

    #[test]
    fn distance_accepts_the_valid_interval() {
        // Just above zero, the default, and the upper bound are all accepted.
        assert_eq!(
            Distance::new(f32::MIN_POSITIVE).unwrap().get(),
            f32::MIN_POSITIVE
        );
        assert_eq!(Distance::new(1.0).unwrap().get(), 1.0);
        assert_eq!(Distance::new(25.0).unwrap().get(), 25.0);
        assert_eq!(Distance::new(0.5).unwrap().get(), 0.5);
    }

    #[test]
    fn distance_rejects_out_of_range_and_non_finite() {
        // 0.0 is rejected on purpose: lossless is a mode, not a distance.
        assert!(Distance::new(0.0).is_err());
        assert!(Distance::new(-1.0).is_err());
        assert!(Distance::new(f32::NAN).is_err());
        assert!(Distance::new(f32::INFINITY).is_err());
        assert!(Distance::new(f32::NEG_INFINITY).is_err());
        // Just over the upper bound is rejected; the bound itself (tested above) is not.
        assert!(Distance::new(25.0001).is_err());
        assert!(Distance::new(f32::MAX).is_err());
    }

    #[test]
    fn distance_default_is_one() {
        assert_eq!(Distance::default().get(), 1.0);
    }

    #[test]
    fn container_default_is_codestream() {
        assert_eq!(Container::default(), Container::Codestream);
    }
}
