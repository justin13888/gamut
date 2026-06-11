//! The TIFF structural model: the byte-order header, Image File Directories (IFDs), and their
//! tag entries.
//!
//! A TIFF file is an 8-byte header (byte-order mark + magic `42` + offset of the first IFD)
//! followed by one or more IFDs. Each IFD is a 2-byte entry count, a sequence of 12-byte entries
//! sorted in ascending tag order, and a 4-byte offset to the next IFD (or `0`). These types model
//! that structure; the reader/writer that serialises them lands in a later phase.

/// Byte order of a TIFF stream, named by the two-byte order mark at the start of the file.
pub enum ByteOrder {
    /// Little-endian (`II`, `0x4949`): least-significant byte first.
    LittleEndian,
    /// Big-endian (`MM`, `0x4D4D`): most-significant byte first.
    BigEndian,
}

/// The type of a tag's value, stored as the 2-byte `Type` of an IFD entry (TIFF 6.0 §2).
///
/// The discriminants match the on-disk codes, e.g. `Short` is `3` and `Rational` is `5`.
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

/// A 16-bit TIFF tag identifier (the `Tag` of an IFD entry), e.g. `256` for `ImageWidth`.
pub struct TagId(pub u16);

/// One entry (field) in an Image File Directory.
///
/// On disk this is 12 bytes: tag (2), field type (2), value count (4), and a value-or-offset
/// word (4) that holds the value inline when it fits in four bytes, or otherwise a file offset
/// to it.
pub struct IfdEntry {
    /// The tag identifying the field.
    pub tag: TagId,
    /// The type of each value.
    pub field_type: FieldType,
    /// The number of values (not bytes) of [`Self::field_type`].
    pub count: u32,
}

/// A parsed Image File Directory — one image (subfile/page) within the TIFF file.
pub struct Ifd {
    /// The directory entries, sorted in ascending tag order.
    pub entries: Vec<IfdEntry>,
}

/// How pixel samples map to colour, stored in the `PhotometricInterpretation` tag (262).
pub enum PhotometricInterpretation {
    /// `0` — bilevel/grayscale where `0` is white and the maximum value is black.
    WhiteIsZero,
    /// `1` — bilevel/grayscale where `0` is black and the maximum value is white.
    BlackIsZero,
    /// `2` — full-colour RGB.
    Rgb,
    /// `3` — palette colour: sample values index a `ColorMap`.
    Palette,
    /// `4` — a transparency mask (an auxiliary bilevel image for another image).
    TransparencyMask,
    /// `5` — CMYK separated colour (TIFF 6.0 §16).
    Cmyk,
    /// `6` — YCbCr colour (TIFF 6.0 §21).
    YCbCr,
    /// `8` — CIE L\*a\*b\* colour (TIFF 6.0 §23).
    CieLab,
}

/// The prediction scheme applied before compression, stored in the `Predictor` tag (317,
/// TIFF 6.0 §14).
pub enum Predictor {
    /// `1` — no prediction.
    None,
    /// `2` — horizontal differencing: each sample is stored as its difference from the sample to
    /// its left.
    HorizontalDifferencing,
}
