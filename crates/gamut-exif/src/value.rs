//! Typed helpers layered over [`gamut_ifd::Value`].
//!
//! EXIF reuses the TIFF field types, so this crate does not wrap [`gamut_ifd::Value`] in a second
//! value enum — it is re-exported unchanged at the crate root. What EXIF adds are the small typed
//! conveniences the raw value model lacks: rational-to-float conversion and text extraction that
//! spans both `ASCII` and the Exif 3.0 `UTF8` string type.

use gamut_ifd::Value;

/// An unsigned rational (`RATIONAL`): a numerator over a denominator.
///
/// A thin, `Copy` view over the `(numerator, denominator)` pair [`gamut_ifd::Value::Rational`]
/// stores, with the float conversion EXIF accessors need. Kept deliberately minimal — no
/// arithmetic or reduction — so the surface stays stable at 1.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rational {
    /// The numerator.
    pub num: u32,
    /// The denominator.
    pub den: u32,
}

impl Rational {
    /// The value as an `f64`, or `None` if the denominator is zero.
    #[must_use]
    pub fn to_f64(self) -> Option<f64> {
        (self.den != 0).then(|| f64::from(self.num) / f64::from(self.den))
    }
}

impl From<(u32, u32)> for Rational {
    fn from((num, den): (u32, u32)) -> Self {
        Self { num, den }
    }
}

impl From<Rational> for (u32, u32) {
    fn from(r: Rational) -> Self {
        (r.num, r.den)
    }
}

/// A signed rational (`SRATIONAL`): a numerator over a denominator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SRational {
    /// The numerator.
    pub num: i32,
    /// The denominator.
    pub den: i32,
}

impl SRational {
    /// The value as an `f64`, or `None` if the denominator is zero.
    #[must_use]
    pub fn to_f64(self) -> Option<f64> {
        (self.den != 0).then(|| f64::from(self.num) / f64::from(self.den))
    }
}

impl From<(i32, i32)> for SRational {
    fn from((num, den): (i32, i32)) -> Self {
        Self { num, den }
    }
}

impl From<SRational> for (i32, i32) {
    fn from(r: SRational) -> Self {
        (r.num, r.den)
    }
}

/// Borrows a tag value as text, covering both `ASCII` (type 2) and Exif 3.0 `UTF8` (type 129)
/// string fields.
///
/// Returns `None` for any non-text value.
#[must_use]
pub fn as_text(value: &Value) -> Option<&str> {
    match value {
        Value::Ascii(s) | Value::Utf8(s) => Some(s.as_str()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rational_to_f64() {
        assert_eq!(Rational { num: 1, den: 2 }.to_f64(), Some(0.5));
        assert_eq!(Rational { num: 1, den: 0 }.to_f64(), None);
        assert_eq!(SRational { num: -1, den: 4 }.to_f64(), Some(-0.25));
        assert_eq!(SRational { num: 5, den: 0 }.to_f64(), None);
    }

    #[test]
    fn rational_tuple_conversions_round_trip() {
        let r = Rational::from((300, 1));
        assert_eq!(r, Rational { num: 300, den: 1 });
        assert_eq!(<(u32, u32)>::from(r), (300, 1));
        let s = SRational::from((-7, 2));
        assert_eq!(s, SRational { num: -7, den: 2 });
        assert_eq!(<(i32, i32)>::from(s), (-7, 2));
    }

    #[test]
    fn as_text_spans_ascii_and_utf8() {
        assert_eq!(as_text(&Value::Ascii("NIKON".into())), Some("NIKON"));
        assert_eq!(as_text(&Value::Utf8("café".into())), Some("café"));
        assert_eq!(as_text(&Value::Short(vec![1])), None);
    }
}
