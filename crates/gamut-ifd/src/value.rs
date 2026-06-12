//! The decoded value of an IFD entry.

/// The decoded value(s) of one IFD entry, one variant per [`crate::FieldType`].
///
/// A TIFF entry always stores a `count` of values of a single type; even a scalar is a 1-element
/// vector here. On disk the values sit inline in the entry's 4-byte value field when they fit, or
/// at a file offset otherwise — a distinction the reader/writer resolve, leaving this type purely
/// the logical value. Variant payload types mirror `gamut-tiff`'s `Value` so the codec can adopt
/// this crate without changing call sites.
pub enum Value {
    /// `BYTE` — unsigned 8-bit integers.
    Byte(Vec<u8>),
    /// `ASCII` — a NUL-terminated 7-bit ASCII string (the terminator is not stored here).
    Ascii(String),
    /// `SHORT` — unsigned 16-bit integers.
    Short(Vec<u16>),
    /// `LONG` — unsigned 32-bit integers.
    Long(Vec<u32>),
    /// `RATIONAL` — unsigned fractions as (numerator, denominator) pairs.
    Rational(Vec<(u32, u32)>),
    /// `SBYTE` — signed 8-bit integers.
    SByte(Vec<i8>),
    /// `UNDEFINED` — raw bytes whose interpretation depends on the field.
    Undefined(Vec<u8>),
    /// `SSHORT` — signed 16-bit integers.
    SShort(Vec<i16>),
    /// `SLONG` — signed 32-bit integers.
    SLong(Vec<i32>),
    /// `SRATIONAL` — signed fractions as (numerator, denominator) pairs.
    SRational(Vec<(i32, i32)>),
    /// `FLOAT` — IEEE single-precision floats.
    Float(Vec<f32>),
    /// `DOUBLE` — IEEE double-precision floats.
    Double(Vec<f64>),
}
