//! The typed value of an EXIF tag.

/// A decoded EXIF tag value — a higher-level, EXIF-typed view over [`gamut_ifd::Value`].
///
/// EXIF reuses the TIFF field types but assigns each tag a specific expected type and meaning; the
/// reader lifts the raw IFD value into one of these and the formatter renders it human-readably
/// (the exiftool-style presentation layer is added during implementation).
pub enum ExifValue {
    /// A text value (an ASCII tag, or a UTF-8 tag in Exif 3.0).
    Text(String),
    /// One or more unsigned integers (`Byte`/`Short`/`Long`).
    Unsigned(Vec<u32>),
    /// One or more signed integers (`SShort`/`SLong`).
    Signed(Vec<i32>),
    /// One or more unsigned rationals (numerator, denominator).
    Rational(Vec<(u32, u32)>),
    /// One or more signed rationals (numerator, denominator).
    SRational(Vec<(i32, i32)>),
    /// Raw `Undefined` bytes whose interpretation is tag-specific (e.g. `MakerNote`, version tags).
    Undefined(Vec<u8>),
}
