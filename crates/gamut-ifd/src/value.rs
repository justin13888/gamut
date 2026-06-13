//! The decoded value of an IFD entry.

use gamut_core::{Error, Result};

use crate::byte_order::ByteOrder;
use crate::types::FieldType;

/// The decoded value(s) of one IFD entry, one variant per [`crate::FieldType`].
///
/// A TIFF entry always stores a `count` of values of a single type; even a scalar is a 1-element
/// vector here. On disk the values sit inline in the entry's value/offset field when they fit, or
/// at a file offset otherwise — a distinction the reader/writer resolve, leaving this type purely
/// the logical value. The BigTIFF 64-bit variants (`Long8`/`SLong8`/`Ifd8`) appear only when the
/// `bigtiff` feature is enabled.
#[derive(Debug, Clone, PartialEq)]
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
    /// `LONG8` — BigTIFF 64-bit unsigned integers.
    #[cfg(feature = "bigtiff")]
    Long8(Vec<u64>),
    /// `SLONG8` — BigTIFF 64-bit signed integers.
    #[cfg(feature = "bigtiff")]
    SLong8(Vec<i64>),
    /// `IFD8` — BigTIFF 64-bit IFD offsets.
    #[cfg(feature = "bigtiff")]
    Ifd8(Vec<u64>),
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
            #[cfg(feature = "bigtiff")]
            Value::Long8(_) => FieldType::Long8,
            #[cfg(feature = "bigtiff")]
            Value::SLong8(_) => FieldType::SLong8,
            #[cfg(feature = "bigtiff")]
            Value::Ifd8(_) => FieldType::Ifd8,
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
            #[cfg(feature = "bigtiff")]
            Value::Long8(v) | Value::Ifd8(v) => v.len(),
            #[cfg(feature = "bigtiff")]
            Value::SLong8(v) => v.len(),
        }
    }

    /// The number of bytes this value occupies on disk (`count * type size`).
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.count() * self.field_type().size()
    }

    /// Coerces a single unsigned-integer value (`BYTE`, `SHORT`, `LONG`, or — with `bigtiff` —
    /// `LONG8`/`IFD8`) to `u32`.
    ///
    /// TIFF readers accept any of these types for an integer field (TIFF 6.0 §2); returns `None`
    /// if the value is not a single unsigned integer or a `LONG8`/`IFD8` exceeds `u32::MAX` (only
    /// possible past the 4 GiB classic-TIFF limit, which an in-memory decode cannot reach anyway).
    #[must_use]
    pub fn as_u32(&self) -> Option<u32> {
        match self {
            Value::Byte(v) if v.len() == 1 => Some(u32::from(v[0])),
            Value::Short(v) if v.len() == 1 => Some(u32::from(v[0])),
            Value::Long(v) if v.len() == 1 => Some(v[0]),
            #[cfg(feature = "bigtiff")]
            Value::Long8(v) | Value::Ifd8(v) if v.len() == 1 => u32::try_from(v[0]).ok(),
            _ => None,
        }
    }

    /// Coerces an array of unsigned integers (`BYTE`, `SHORT`, `LONG`, or — with `bigtiff` —
    /// `LONG8`/`IFD8`) to `Vec<u32>`.
    ///
    /// Returns `None` for any other type, or if a `LONG8`/`IFD8` element exceeds `u32::MAX`. This
    /// lets a decoder read BigTIFF `StripOffsets`/`StripByteCounts`, which libtiff writes as
    /// `LONG8`.
    #[must_use]
    pub fn as_u32_vec(&self) -> Option<Vec<u32>> {
        match self {
            Value::Byte(v) => Some(v.iter().map(|&x| u32::from(x)).collect()),
            Value::Short(v) => Some(v.iter().map(|&x| u32::from(x)).collect()),
            Value::Long(v) => Some(v.clone()),
            #[cfg(feature = "bigtiff")]
            Value::Long8(v) | Value::Ifd8(v) => v.iter().map(|&x| u32::try_from(x).ok()).collect(),
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
            #[cfg(feature = "bigtiff")]
            Value::Long8(v) | Value::Ifd8(v) => {
                for &x in v {
                    out.extend_from_slice(&order.pack_u64(x));
                }
            }
            #[cfg(feature = "bigtiff")]
            Value::SLong8(v) => {
                for &x in v {
                    out.extend_from_slice(&order.pack_u64(x as u64));
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
        #[cfg(feature = "bigtiff")]
        let u64s = |b: &[u8]| -> Vec<u64> {
            b.chunks_exact(8)
                .map(|c| order.u64([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
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
            #[cfg(feature = "bigtiff")]
            FieldType::Long8 => Value::Long8(u64s(bytes)),
            #[cfg(feature = "bigtiff")]
            FieldType::Ifd8 => Value::Ifd8(u64s(bytes)),
            #[cfg(feature = "bigtiff")]
            FieldType::SLong8 => Value::SLong8(u64s(bytes).into_iter().map(|x| x as i64).collect()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            #[cfg(feature = "bigtiff")]
            {
                value_roundtrip(
                    Value::Long8(vec![0x0123_4567_89AB_CDEF, 0, u64::MAX]),
                    order,
                );
                value_roundtrip(Value::SLong8(vec![-1, i64::MIN, 42]), order);
                value_roundtrip(Value::Ifd8(vec![16, 0x1_0000_0000]), order);
            }
        }
    }

    #[test]
    fn count_and_byte_len_are_exact() {
        // `count` / `byte_len` are otherwise only an internal `Vec::with_capacity` hint and a
        // round-trip-tolerated length, so pin them directly. ASCII counts the trailing NUL.
        assert_eq!(Value::Ascii("gamut".into()).count(), 6);
        assert_eq!(Value::Short(vec![1, 2, 3]).count(), 3);
        assert_eq!(Value::Short(vec![1, 2, 3]).byte_len(), 6); // 3 * 2
        assert_eq!(Value::Ascii("ab".into()).byte_len(), 3); // 3 * 1
        assert_eq!(Value::Rational(vec![(1, 2)]).byte_len(), 8); // 1 * 8
    }

    #[test]
    fn integer_coercion_accepts_byte_short_long() {
        assert_eq!(Value::Byte(vec![5]).as_u32(), Some(5));
        assert_eq!(Value::Short(vec![300]).as_u32(), Some(300));
        assert_eq!(Value::Long(vec![70000]).as_u32(), Some(70000));
        // Multi-element values are not scalars: each type's `v.len() == 1` guard must reject them
        // (not just SHORT).
        assert_eq!(Value::Byte(vec![1, 2]).as_u32(), None);
        assert_eq!(Value::Short(vec![1, 2]).as_u32(), None);
        assert_eq!(Value::Long(vec![1, 2]).as_u32(), None);
        assert_eq!(Value::Ascii("x".into()).as_u32(), None);
        // Every accepted vector type round-trips through as_u32_vec, not only SHORT.
        assert_eq!(Value::Byte(vec![1, 2, 3]).as_u32_vec(), Some(vec![1, 2, 3]));
        assert_eq!(
            Value::Short(vec![1, 2, 3]).as_u32_vec(),
            Some(vec![1, 2, 3])
        );
        assert_eq!(Value::Long(vec![7, 8]).as_u32_vec(), Some(vec![7, 8]));
        assert_eq!(Value::Ascii("x".into()).as_u32_vec(), None);
    }

    /// BigTIFF `LONG8`/`IFD8` coerce to `u32` when in range, so a decoder reads 64-bit offsets;
    /// out-of-range values fail cleanly rather than truncating.
    #[cfg(feature = "bigtiff")]
    #[test]
    fn integer_coercion_accepts_bigtiff_64bit() {
        assert_eq!(Value::Long8(vec![70000]).as_u32(), Some(70000));
        assert_eq!(Value::Ifd8(vec![8, 1024]).as_u32_vec(), Some(vec![8, 1024]));
        assert_eq!(Value::Long8(vec![0x1_0000_0000]).as_u32(), None);
        assert_eq!(Value::Long8(vec![1, 0x1_0000_0000]).as_u32_vec(), None);
        // A multi-element (but in-range) value still isn't a scalar — pins the `v.len() == 1` guard
        // rather than the out-of-range path above.
        assert_eq!(Value::Long8(vec![1, 2]).as_u32(), None);
    }

    #[test]
    fn decode_rejects_truncated_value() {
        // A LONG needs 4 bytes; only 2 are supplied.
        assert!(Value::decode(FieldType::Long, 1, &[0, 0], ByteOrder::LittleEndian).is_err());
    }
}
