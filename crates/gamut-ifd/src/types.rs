//! The field types an IFD entry value can take.

/// The type of a tag's value, stored as the 2-byte `Type` of an IFD entry (TIFF 6.0 §2).
///
/// The discriminants match the on-disk codes (`Byte` is `1`, `Short` is `3`, `Rational` is `5`,
/// …). These twelve are the TIFF 6.0 field types; the BigTIFF 64-bit additions (`Long8`/`SLong8`/
/// `Ifd8`) are out of the current scaffold's scope (see `STATUS.md`, gated behind a future
/// `bigtiff` feature so the set stays additive).
pub enum FieldType {
    /// `1` — an 8-bit unsigned integer.
    Byte,
    /// `2` — an 8-bit byte holding a 7-bit ASCII code; NUL-terminated.
    Ascii,
    /// `3` — a 16-bit unsigned integer.
    Short,
    /// `4` — a 32-bit unsigned integer.
    Long,
    /// `5` — two `Long`s: a fraction's numerator then denominator.
    Rational,
    /// `6` — an 8-bit signed (two's-complement) integer.
    SByte,
    /// `7` — an 8-bit byte whose meaning depends on the field.
    Undefined,
    /// `8` — a 16-bit signed (two's-complement) integer.
    SShort,
    /// `9` — a 32-bit signed (two's-complement) integer.
    SLong,
    /// `10` — two `SLong`s: a signed fraction's numerator then denominator.
    SRational,
    /// `11` — a 32-bit IEEE single-precision float.
    Float,
    /// `12` — a 64-bit IEEE double-precision float.
    Double,
}
