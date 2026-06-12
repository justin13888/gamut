//! The TIFF structural model: the byte-order header, Image File Directories (IFDs), and their
//! tag entries.
//!
//! A TIFF file is an 8-byte header (byte-order mark + magic `42` + the offset of the first IFD)
//! followed by one or more IFDs. Each IFD is a 2-byte entry count, a sequence of 12-byte entries
//! sorted in ascending tag order, and a 4-byte offset to the next IFD (or `0`). An entry carries
//! its value inline when it fits in four bytes, or otherwise a file offset to it.
//!
//! These types model that structure; [`crate::reader`] parses it and [`crate::writer`] serialises
//! it.

use gamut_core::{Error, Result};

/// Byte order of a TIFF stream, named by the two-byte order mark at the start of the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ByteOrder {
    /// Little-endian (`II`, `0x4949`): least-significant byte first.
    LittleEndian,
    /// Big-endian (`MM`, `0x4D4D`): most-significant byte first.
    BigEndian,
}

impl ByteOrder {
    /// Decodes a 16-bit unsigned integer from two bytes in this order.
    #[must_use]
    pub fn u16(self, b: [u8; 2]) -> u16 {
        match self {
            ByteOrder::LittleEndian => u16::from_le_bytes(b),
            ByteOrder::BigEndian => u16::from_be_bytes(b),
        }
    }

    /// Decodes a 32-bit unsigned integer from four bytes in this order.
    #[must_use]
    pub fn u32(self, b: [u8; 4]) -> u32 {
        match self {
            ByteOrder::LittleEndian => u32::from_le_bytes(b),
            ByteOrder::BigEndian => u32::from_be_bytes(b),
        }
    }

    /// Encodes a 16-bit unsigned integer to two bytes in this order.
    #[must_use]
    pub fn pack_u16(self, v: u16) -> [u8; 2] {
        match self {
            ByteOrder::LittleEndian => v.to_le_bytes(),
            ByteOrder::BigEndian => v.to_be_bytes(),
        }
    }

    /// Encodes a 32-bit unsigned integer to four bytes in this order.
    #[must_use]
    pub fn pack_u32(self, v: u32) -> [u8; 4] {
        match self {
            ByteOrder::LittleEndian => v.to_le_bytes(),
            ByteOrder::BigEndian => v.to_be_bytes(),
        }
    }
}

/// The type of a tag's value, stored as the 2-byte `Type` of an IFD entry (TIFF 6.0 §2).
///
/// The discriminants match the on-disk codes, e.g. `Short` is `3` and `Rational` is `5`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl FieldType {
    /// Returns the field type for an on-disk type code, or `None` for an unknown code.
    #[must_use]
    pub fn from_code(code: u16) -> Option<Self> {
        Some(match code {
            1 => FieldType::Byte,
            2 => FieldType::Ascii,
            3 => FieldType::Short,
            4 => FieldType::Long,
            5 => FieldType::Rational,
            6 => FieldType::SByte,
            7 => FieldType::Undefined,
            8 => FieldType::SShort,
            9 => FieldType::SLong,
            10 => FieldType::SRational,
            11 => FieldType::Float,
            12 => FieldType::Double,
            _ => return None,
        })
    }

    /// Returns the on-disk type code.
    #[must_use]
    pub fn code(self) -> u16 {
        match self {
            FieldType::Byte => 1,
            FieldType::Ascii => 2,
            FieldType::Short => 3,
            FieldType::Long => 4,
            FieldType::Rational => 5,
            FieldType::SByte => 6,
            FieldType::Undefined => 7,
            FieldType::SShort => 8,
            FieldType::SLong => 9,
            FieldType::SRational => 10,
            FieldType::Float => 11,
            FieldType::Double => 12,
        }
    }

    /// Returns the number of bytes in a single value of this type.
    #[must_use]
    pub fn size(self) -> usize {
        match self {
            FieldType::Byte | FieldType::Ascii | FieldType::SByte | FieldType::Undefined => 1,
            FieldType::Short | FieldType::SShort => 2,
            FieldType::Long | FieldType::SLong | FieldType::Float => 4,
            FieldType::Rational | FieldType::SRational | FieldType::Double => 8,
        }
    }
}

/// The decoded value of a TIFF field — always an array, even for a single value (TIFF 6.0 §2).
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `BYTE` values.
    Byte(Vec<u8>),
    /// An `ASCII` string, without its trailing NUL.
    Ascii(String),
    /// `SHORT` values.
    Short(Vec<u16>),
    /// `LONG` values.
    Long(Vec<u32>),
    /// `RATIONAL` values (numerator, denominator).
    Rational(Vec<(u32, u32)>),
    /// `SBYTE` values.
    SByte(Vec<i8>),
    /// `UNDEFINED` bytes.
    Undefined(Vec<u8>),
    /// `SSHORT` values.
    SShort(Vec<i16>),
    /// `SLONG` values.
    SLong(Vec<i32>),
    /// `SRATIONAL` values (numerator, denominator).
    SRational(Vec<(i32, i32)>),
    /// `FLOAT` values.
    Float(Vec<f32>),
    /// `DOUBLE` values.
    Double(Vec<f64>),
}

impl Value {
    /// The field type of this value.
    #[must_use]
    pub fn field_type(&self) -> FieldType {
        match self {
            Value::Byte(_) => FieldType::Byte,
            Value::Ascii(_) => FieldType::Ascii,
            Value::Short(_) => FieldType::Short,
            Value::Long(_) => FieldType::Long,
            Value::Rational(_) => FieldType::Rational,
            Value::SByte(_) => FieldType::SByte,
            Value::Undefined(_) => FieldType::Undefined,
            Value::SShort(_) => FieldType::SShort,
            Value::SLong(_) => FieldType::SLong,
            Value::SRational(_) => FieldType::SRational,
            Value::Float(_) => FieldType::Float,
            Value::Double(_) => FieldType::Double,
        }
    }

    /// The `Count` of this value: the number of elements, or for `ASCII` the number of bytes
    /// including the terminating NUL.
    #[must_use]
    pub fn count(&self) -> usize {
        match self {
            Value::Byte(v) | Value::Undefined(v) => v.len(),
            Value::Ascii(s) => s.len() + 1,
            Value::Short(v) => v.len(),
            Value::Long(v) => v.len(),
            Value::Rational(v) => v.len(),
            Value::SByte(v) => v.len(),
            Value::SShort(v) => v.len(),
            Value::SLong(v) => v.len(),
            Value::SRational(v) => v.len(),
            Value::Float(v) => v.len(),
            Value::Double(v) => v.len(),
        }
    }

    /// The number of bytes this value occupies on disk (`count * type size`).
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.count() * self.field_type().size()
    }

    /// Coerces a single unsigned-integer value (`BYTE`, `SHORT`, or `LONG`) to `u32`.
    ///
    /// TIFF readers accept any of these types for an integer field (TIFF 6.0 §2); returns `None`
    /// if the value is not a single unsigned integer.
    #[must_use]
    pub fn as_u32(&self) -> Option<u32> {
        match self {
            Value::Byte(v) if v.len() == 1 => Some(u32::from(v[0])),
            Value::Short(v) if v.len() == 1 => Some(u32::from(v[0])),
            Value::Long(v) if v.len() == 1 => Some(v[0]),
            _ => None,
        }
    }

    /// Coerces an array of unsigned integers (`BYTE`, `SHORT`, or `LONG`) to `Vec<u32>`.
    #[must_use]
    pub fn as_u32_vec(&self) -> Option<Vec<u32>> {
        match self {
            Value::Byte(v) => Some(v.iter().map(|&x| u32::from(x)).collect()),
            Value::Short(v) => Some(v.iter().map(|&x| u32::from(x)).collect()),
            Value::Long(v) => Some(v.clone()),
            _ => None,
        }
    }

    /// Serialises the value's elements to bytes in `order` (without any inline/offset padding).
    #[must_use]
    pub fn encode(&self, order: ByteOrder) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.byte_len());
        match self {
            Value::Byte(v) | Value::Undefined(v) => out.extend_from_slice(v),
            Value::Ascii(s) => {
                out.extend_from_slice(s.as_bytes());
                out.push(0);
            }
            Value::SByte(v) => out.extend(v.iter().map(|&x| x as u8)),
            Value::Short(v) => {
                for &x in v {
                    out.extend_from_slice(&order.pack_u16(x));
                }
            }
            Value::SShort(v) => {
                for &x in v {
                    out.extend_from_slice(&order.pack_u16(x as u16));
                }
            }
            Value::Long(v) => {
                for &x in v {
                    out.extend_from_slice(&order.pack_u32(x));
                }
            }
            Value::SLong(v) => {
                for &x in v {
                    out.extend_from_slice(&order.pack_u32(x as u32));
                }
            }
            Value::Float(v) => {
                for &x in v {
                    out.extend_from_slice(&order.pack_u32(x.to_bits()));
                }
            }
            Value::Rational(v) => {
                for &(n, d) in v {
                    out.extend_from_slice(&order.pack_u32(n));
                    out.extend_from_slice(&order.pack_u32(d));
                }
            }
            Value::SRational(v) => {
                for &(n, d) in v {
                    out.extend_from_slice(&order.pack_u32(n as u32));
                    out.extend_from_slice(&order.pack_u32(d as u32));
                }
            }
            Value::Double(v) => {
                for &x in v {
                    let b = x.to_bits();
                    let lo = order.pack_u32(b as u32);
                    let hi = order.pack_u32((b >> 32) as u32);
                    match order {
                        ByteOrder::LittleEndian => {
                            out.extend_from_slice(&lo);
                            out.extend_from_slice(&hi);
                        }
                        ByteOrder::BigEndian => {
                            out.extend_from_slice(&hi);
                            out.extend_from_slice(&lo);
                        }
                    }
                }
            }
        }
        out
    }

    /// Parses `count` values of `ty` from `bytes` (which must hold at least `count * ty.size()`
    /// bytes) in `order`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `bytes` is too short for the declared count.
    pub fn decode(ty: FieldType, count: usize, bytes: &[u8], order: ByteOrder) -> Result<Value> {
        let need = count
            .checked_mul(ty.size())
            .ok_or(Error::InvalidInput("TIFF: field length overflow"))?;
        let bytes = bytes
            .get(..need)
            .ok_or(Error::InvalidInput("TIFF: field value out of bounds"))?;
        let u16s =
            |b: &[u8]| -> Vec<u16> { b.chunks_exact(2).map(|c| order.u16([c[0], c[1]])).collect() };
        let u32s = |b: &[u8]| -> Vec<u32> {
            b.chunks_exact(4)
                .map(|c| order.u32([c[0], c[1], c[2], c[3]]))
                .collect()
        };
        Ok(match ty {
            FieldType::Byte => Value::Byte(bytes.to_vec()),
            FieldType::Undefined => Value::Undefined(bytes.to_vec()),
            FieldType::SByte => Value::SByte(bytes.iter().map(|&x| x as i8).collect()),
            FieldType::Ascii => {
                let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
                let s = core::str::from_utf8(&bytes[..end])
                    .map_err(|_| Error::InvalidInput("TIFF: non-UTF-8 ASCII field"))?;
                Value::Ascii(s.to_owned())
            }
            FieldType::Short => Value::Short(u16s(bytes)),
            FieldType::SShort => Value::SShort(u16s(bytes).into_iter().map(|x| x as i16).collect()),
            FieldType::Long => Value::Long(u32s(bytes)),
            FieldType::SLong => Value::SLong(u32s(bytes).into_iter().map(|x| x as i32).collect()),
            FieldType::Float => Value::Float(u32s(bytes).into_iter().map(f32::from_bits).collect()),
            FieldType::Rational => {
                let w = u32s(bytes);
                Value::Rational(w.chunks_exact(2).map(|c| (c[0], c[1])).collect())
            }
            FieldType::SRational => {
                let w = u32s(bytes);
                Value::SRational(
                    w.chunks_exact(2)
                        .map(|c| (c[0] as i32, c[1] as i32))
                        .collect(),
                )
            }
            FieldType::Double => {
                let mut v = Vec::with_capacity(count);
                for c in bytes.chunks_exact(8) {
                    let (a, b) = (
                        order.u32([c[0], c[1], c[2], c[3]]),
                        order.u32([c[4], c[5], c[6], c[7]]),
                    );
                    let bits = match order {
                        ByteOrder::LittleEndian => u64::from(a) | (u64::from(b) << 32),
                        ByteOrder::BigEndian => u64::from(b) | (u64::from(a) << 32),
                    };
                    v.push(f64::from_bits(bits));
                }
                Value::Double(v)
            }
        })
    }
}

/// One field (entry) of an Image File Directory: a tag and its value.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// The tag identifying the field (see [`crate::tags`]).
    pub tag: u16,
    /// The field's value.
    pub value: Value,
}

/// A parsed Image File Directory — one image (subfile/page) within the TIFF file.
///
/// Fields are kept sorted in ascending tag order, as required on disk (TIFF 6.0 §2).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ifd {
    fields: Vec<Field>,
}

impl Ifd {
    /// Creates an empty directory.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the directory's fields, sorted by ascending tag.
    #[must_use]
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    /// Returns the value of `tag`, or `None` if absent.
    #[must_use]
    pub fn get(&self, tag: u16) -> Option<&Value> {
        self.fields
            .binary_search_by_key(&tag, |f| f.tag)
            .ok()
            .map(|i| &self.fields[i].value)
    }

    /// Returns `tag` coerced to a single `u32` (accepting `BYTE`/`SHORT`/`LONG`).
    #[must_use]
    pub fn get_u32(&self, tag: u16) -> Option<u32> {
        self.get(tag).and_then(Value::as_u32)
    }

    /// Returns `tag` coerced to a `Vec<u32>` (accepting `BYTE`/`SHORT`/`LONG`).
    #[must_use]
    pub fn get_u32_vec(&self, tag: u16) -> Option<Vec<u32>> {
        self.get(tag).and_then(Value::as_u32_vec)
    }

    /// Inserts or replaces the value of `tag`, keeping the fields sorted.
    pub fn set(&mut self, tag: u16, value: Value) {
        match self.fields.binary_search_by_key(&tag, |f| f.tag) {
            Ok(i) => self.fields[i].value = value,
            Err(i) => self.fields.insert(i, Field { tag, value }),
        }
    }
}

/// How pixel samples map to colour, stored in the `PhotometricInterpretation` tag (262).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl PhotometricInterpretation {
    /// Returns the interpretation for an on-disk tag value, or `None` if unrecognised.
    #[must_use]
    pub fn from_code(code: u32) -> Option<Self> {
        Some(match code {
            0 => PhotometricInterpretation::WhiteIsZero,
            1 => PhotometricInterpretation::BlackIsZero,
            2 => PhotometricInterpretation::Rgb,
            3 => PhotometricInterpretation::Palette,
            4 => PhotometricInterpretation::TransparencyMask,
            5 => PhotometricInterpretation::Cmyk,
            6 => PhotometricInterpretation::YCbCr,
            8 => PhotometricInterpretation::CieLab,
            _ => return None,
        })
    }

    /// Returns the on-disk tag value.
    #[must_use]
    pub fn code(self) -> u16 {
        match self {
            PhotometricInterpretation::WhiteIsZero => 0,
            PhotometricInterpretation::BlackIsZero => 1,
            PhotometricInterpretation::Rgb => 2,
            PhotometricInterpretation::Palette => 3,
            PhotometricInterpretation::TransparencyMask => 4,
            PhotometricInterpretation::Cmyk => 5,
            PhotometricInterpretation::YCbCr => 6,
            PhotometricInterpretation::CieLab => 8,
        }
    }
}

/// The prediction scheme applied before compression, stored in the `Predictor` tag (317,
/// TIFF 6.0 §14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Predictor {
    /// `1` — no prediction.
    #[default]
    None,
    /// `2` — horizontal differencing: each sample is stored as its difference from the sample to
    /// its left.
    HorizontalDifferencing,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_order_roundtrips_integers() {
        for order in [ByteOrder::LittleEndian, ByteOrder::BigEndian] {
            assert_eq!(order.u16(order.pack_u16(0xABCD)), 0xABCD);
            assert_eq!(order.u32(order.pack_u32(0x0123_4567)), 0x0123_4567);
        }
        assert_eq!(ByteOrder::LittleEndian.pack_u16(0x00FF), [0xFF, 0x00]);
        assert_eq!(ByteOrder::BigEndian.pack_u16(0x00FF), [0x00, 0xFF]);
    }

    #[test]
    fn field_type_codes_round_trip() {
        for code in 1..=12u16 {
            let ty = FieldType::from_code(code).expect("known type");
            assert_eq!(ty.code(), code);
        }
        assert_eq!(FieldType::from_code(0), None);
        assert_eq!(FieldType::from_code(13), None);
        assert_eq!(FieldType::Rational.size(), 8);
        assert_eq!(FieldType::Short.size(), 2);
    }

    fn value_roundtrip(value: Value, order: ByteOrder) {
        let bytes = value.encode(order);
        let decoded =
            Value::decode(value.field_type(), value.count(), &bytes, order).expect("decode");
        assert_eq!(decoded, value);
    }

    #[test]
    fn values_roundtrip_in_both_orders() {
        for order in [ByteOrder::LittleEndian, ByteOrder::BigEndian] {
            value_roundtrip(Value::Byte(vec![1, 2, 3]), order);
            value_roundtrip(Value::Ascii("gamut".to_owned()), order);
            value_roundtrip(Value::Short(vec![256, 257, 0xFFFF]), order);
            value_roundtrip(Value::Long(vec![0xDEAD_BEEF, 7]), order);
            value_roundtrip(Value::Rational(vec![(300, 1), (72, 1)]), order);
            value_roundtrip(Value::SByte(vec![-1, 2, -128]), order);
            value_roundtrip(Value::SShort(vec![-1, 30000]), order);
            value_roundtrip(Value::SLong(vec![-1, i32::MIN]), order);
            value_roundtrip(Value::SRational(vec![(-1, 2)]), order);
            value_roundtrip(Value::Float(vec![1.5, -0.25]), order);
            value_roundtrip(Value::Double(vec![1.5, -0.0625]), order);
            value_roundtrip(Value::Undefined(vec![0, 255, 7]), order);
        }
    }

    #[test]
    fn integer_coercion_accepts_byte_short_long() {
        assert_eq!(Value::Byte(vec![5]).as_u32(), Some(5));
        assert_eq!(Value::Short(vec![300]).as_u32(), Some(300));
        assert_eq!(Value::Long(vec![70000]).as_u32(), Some(70000));
        assert_eq!(Value::Short(vec![1, 2]).as_u32(), None);
        assert_eq!(Value::Ascii("x".into()).as_u32(), None);
        assert_eq!(
            Value::Short(vec![1, 2, 3]).as_u32_vec(),
            Some(vec![1, 2, 3])
        );
    }

    #[test]
    fn ifd_keeps_fields_sorted_and_replaces() {
        use crate::tags;
        let mut ifd = Ifd::new();
        ifd.set(tags::COMPRESSION, Value::Short(vec![1]));
        ifd.set(tags::IMAGE_WIDTH, Value::Short(vec![4]));
        ifd.set(tags::IMAGE_LENGTH, Value::Short(vec![3]));
        let order: Vec<u16> = ifd.fields().iter().map(|f| f.tag).collect();
        assert_eq!(order, vec![256, 257, 259]);
        ifd.set(tags::IMAGE_WIDTH, Value::Short(vec![8]));
        assert_eq!(ifd.get_u32(tags::IMAGE_WIDTH), Some(8));
        assert_eq!(ifd.fields().len(), 3);
    }
}
