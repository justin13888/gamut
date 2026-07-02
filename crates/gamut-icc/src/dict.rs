//! `dictType` (ICC.1:2022 §10.9): a dictionary of name→value pairs, each optionally carrying
//! localized display strings.
//!
//! Names and values are UTF-16BE strings (not NUL-terminated); the optional display name/value are
//! `multiLocalizedUnicodeType` elements. All four items are addressed by `(offset, size)` pairs in a
//! fixed-size record, with the string/mluc storage packed 4-byte-aligned after the record table.
//!
//! The on-disk record size (16/24/32 bytes) selects how many items a record can carry; it is
//! recomputed on write from which display elements are present. A display item marked "present but
//! not for display" (a nonzero offset with a zero size, §10.9) is decoded as absent — the one dict
//! nuance gamut-icc does not round-trip distinctly.

use gamut_core::{Error, Result};

use crate::bytes::{ByteReader, pad_to_4};
use crate::mluc::{Mluc, decode_mluc, encode_mluc};

/// One name→value record of a [`Dict`] (§10.9 Table 39).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictEntry {
    /// The name string (required, non-empty).
    pub name: String,
    /// The value string, if present (`None` when the value offset is zero).
    pub value: Option<String>,
    /// The localized display form of the name, if present.
    pub display_name: Option<Mluc>,
    /// The localized display form of the value, if present.
    pub display_value: Option<Mluc>,
}

/// A `dictType` element (§10.9): a metadata dictionary of unique-named records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dict {
    /// The name→value records.
    pub entries: Vec<DictEntry>,
}

/// Decodes a `dictType` element.
pub(crate) fn decode_dict(element: &[u8]) -> Result<Dict> {
    let mut r = ByteReader::at(element, 8)?;
    let count = r.u32()? as usize;
    let record_size = r.u32()? as usize;
    if !matches!(record_size, 16 | 24 | 32) {
        return Err(Error::InvalidInput(
            "icc: dict record size not 16, 24, or 32",
        ));
    }
    // Bound the record table (starts at byte 16) against the element before iterating.
    if count
        .checked_mul(record_size)
        .and_then(|n| n.checked_add(16))
        .is_none_or(|end| end > element.len())
    {
        return Err(Error::InvalidInput(
            "icc: dict record table exceeds element",
        ));
    }
    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let mut rr = ByteReader::at(element, 16 + i * record_size)?;
        let name = decode_string(element, rr.u32()? as usize, rr.u32()? as usize)?.ok_or(
            Error::InvalidInput("icc: dict record without a name string"),
        )?;
        let value = decode_string(element, rr.u32()? as usize, rr.u32()? as usize)?;
        let display_name = if record_size >= 24 {
            decode_display(element, rr.u32()? as usize, rr.u32()? as usize)?
        } else {
            None
        };
        let display_value = if record_size == 32 {
            decode_display(element, rr.u32()? as usize, rr.u32()? as usize)?
        } else {
            None
        };
        entries.push(DictEntry {
            name,
            value,
            display_name,
            display_value,
        });
    }
    Ok(Dict { entries })
}

/// Decodes a UTF-16BE string item; a zero offset means the item is absent.
fn decode_string(element: &[u8], offset: usize, size: usize) -> Result<Option<String>> {
    if offset == 0 {
        return Ok(None);
    }
    let end = offset
        .checked_add(size)
        .ok_or(Error::InvalidInput("icc: dict string overflow"))?;
    let bytes = element
        .get(offset..end)
        .ok_or(Error::InvalidInput("icc: dict string out of bounds"))?;
    Ok(Some(decode_utf16be(bytes)?))
}

/// Decodes an optional `multiLocalizedUnicodeType` display item; a zero offset or size means absent
/// (the latter folds the rare "present but not for display" marker into absence).
fn decode_display(element: &[u8], offset: usize, size: usize) -> Result<Option<Mluc>> {
    if offset == 0 || size == 0 {
        return Ok(None);
    }
    let end = offset
        .checked_add(size)
        .ok_or(Error::InvalidInput("icc: dict display element overflow"))?;
    let bytes = element.get(offset..end).ok_or(Error::InvalidInput(
        "icc: dict display element out of bounds",
    ))?;
    Ok(Some(decode_mluc(bytes)?))
}

/// Decodes UTF-16BE bytes into a `String` (dict strings are not NUL-terminated, so nothing is
/// trimmed).
fn decode_utf16be(bytes: &[u8]) -> Result<String> {
    if !bytes.len().is_multiple_of(2) {
        return Err(Error::InvalidInput("icc: odd-length UTF-16 dict string"));
    }
    let units = bytes
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]));
    let mut text = String::new();
    for unit in char::decode_utf16(units) {
        text.push(unit.map_err(|_| Error::InvalidInput("icc: invalid UTF-16 dict string"))?);
    }
    Ok(text)
}

/// Serializes a `dictType` element (the inverse of [`decode_dict`]). The record size is chosen as
/// the smallest of 16/24/32 that can carry the display items actually present.
pub(crate) fn encode_dict(dict: &Dict, out: &mut Vec<u8>) {
    let has_display_value = dict.entries.iter().any(|e| e.display_value.is_some());
    let has_display_name = dict.entries.iter().any(|e| e.display_name.is_some());
    let record_size = if has_display_value {
        32
    } else if has_display_name {
        24
    } else {
        16
    };

    out.extend_from_slice(b"dict");
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(dict.entries.len() as u32).to_be_bytes());
    out.extend_from_slice(&(record_size as u32).to_be_bytes());

    let storage_start = 16 + dict.entries.len() * record_size;
    let mut storage = Vec::new();
    let mut records = Vec::new();
    for entry in &dict.entries {
        let name = store_string(&mut storage, storage_start, Some(&entry.name));
        let value = store_string(&mut storage, storage_start, entry.value.as_deref());
        let display_name = store_mluc(&mut storage, storage_start, entry.display_name.as_ref());
        let display_value = store_mluc(&mut storage, storage_start, entry.display_value.as_ref());
        records.push((name, value, display_name, display_value));
    }

    for (name, value, display_name, display_value) in records {
        push_range(out, name);
        push_range(out, value);
        if record_size >= 24 {
            push_range(out, display_name);
        }
        if record_size == 32 {
            push_range(out, display_value);
        }
    }
    out.extend_from_slice(&storage);
}

/// Appends a UTF-16BE string to the 4-byte-aligned storage area, returning its `(offset, size)`
/// relative to the element start. `None` yields the absent marker `(0, 0)`.
fn store_string(storage: &mut Vec<u8>, storage_start: usize, text: Option<&str>) -> (u32, u32) {
    let Some(text) = text else {
        return (0, 0);
    };
    pad_to_4(storage);
    let offset = storage_start + storage.len();
    let start = storage.len();
    for unit in text.encode_utf16() {
        storage.extend_from_slice(&unit.to_be_bytes());
    }
    (offset as u32, (storage.len() - start) as u32)
}

/// Appends a `multiLocalizedUnicodeType` display element to the storage area, returning its
/// `(offset, size)`. `None` yields the absent marker `(0, 0)`.
fn store_mluc(storage: &mut Vec<u8>, storage_start: usize, mluc: Option<&Mluc>) -> (u32, u32) {
    let Some(mluc) = mluc else {
        return (0, 0);
    };
    pad_to_4(storage);
    let offset = storage_start + storage.len();
    let start = storage.len();
    encode_mluc(mluc, storage);
    (offset as u32, (storage.len() - start) as u32)
}

/// Appends an `(offset, size)` pair as two big-endian `uInt32`.
fn push_range(out: &mut Vec<u8>, (offset, size): (u32, u32)) {
    out.extend_from_slice(&offset.to_be_bytes());
    out.extend_from_slice(&size.to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mluc::MlucRecord;

    #[test]
    fn dict_with_exactly_sized_record_table_decodes() {
        // One 16-byte record filling the element exactly (the name offset points back into the
        // reserved header bytes): the table bound rejects only a table *exceeding* the element,
        // so an exact fit is valid.
        let mut e = b"dict\x00\x00\x00\x00".to_vec();
        e.extend_from_slice(&1u32.to_be_bytes()); // record count
        e.extend_from_slice(&16u32.to_be_bytes()); // record size
        e.extend_from_slice(&4u32.to_be_bytes()); // name offset → the reserved bytes
        e.extend_from_slice(&2u32.to_be_bytes()); // name size (one UTF-16 unit)
        e.extend_from_slice(&0u32.to_be_bytes()); // value offset: absent
        e.extend_from_slice(&0u32.to_be_bytes()); // value size
        assert_eq!(e.len(), 32); // 16 + 1 × 16 == element length
        let dict = decode_dict(&e).unwrap();
        assert_eq!(dict.entries.len(), 1);
        assert_eq!(dict.entries[0].value, None);
    }

    #[test]
    fn dict_display_marker_decodes_as_absent() {
        // §10.9: a display item with a nonzero offset and zero size marks "present but not for
        // display"; gamut-icc folds it to absent (the documented leniency) instead of erroring.
        let mut e = b"dict\x00\x00\x00\x00".to_vec();
        e.extend_from_slice(&1u32.to_be_bytes()); // record count
        e.extend_from_slice(&24u32.to_be_bytes()); // record size (with display names)
        e.extend_from_slice(&40u32.to_be_bytes()); // name offset → storage after the table
        e.extend_from_slice(&2u32.to_be_bytes()); // name size
        e.extend_from_slice(&0u32.to_be_bytes()); // value offset: absent
        e.extend_from_slice(&0u32.to_be_bytes()); // value size
        e.extend_from_slice(&40u32.to_be_bytes()); // display-name offset: nonzero…
        e.extend_from_slice(&0u32.to_be_bytes()); // …with zero size: the marker
        e.extend_from_slice(&[0x00, 0x6B]); // "k"
        let dict = decode_dict(&e).unwrap();
        assert_eq!(dict.entries[0].name, "k");
        assert_eq!(dict.entries[0].display_name, None);
    }

    fn en_us(text: &str) -> Mluc {
        Mluc {
            records: vec![MlucRecord {
                language: *b"en",
                country: *b"US",
                text: text.to_owned(),
            }],
        }
    }

    #[test]
    fn round_trips_names_and_values_only() {
        // No display elements → a 16-byte record layout.
        let dict = Dict {
            entries: vec![
                DictEntry {
                    name: "Author".to_owned(),
                    value: Some("Ada".to_owned()),
                    display_name: None,
                    display_value: None,
                },
                DictEntry {
                    name: "Empty".to_owned(),
                    value: Some(String::new()), // present-but-empty, distinct from absent
                    display_name: None,
                    display_value: None,
                },
                DictEntry {
                    name: "NoValue".to_owned(),
                    value: None,
                    display_name: None,
                    display_value: None,
                },
            ],
        };
        let mut out = Vec::new();
        encode_dict(&dict, &mut out);
        // 16-byte records: header (16) + 3×16 record table = 64, then storage.
        assert_eq!(&out[12..16], &16u32.to_be_bytes());
        assert_eq!(decode_dict(&out).unwrap(), dict);
    }

    #[test]
    fn round_trips_with_display_elements() {
        // A display_value forces the 32-byte record layout.
        let dict = Dict {
            entries: vec![DictEntry {
                name: "Key".to_owned(),
                value: Some("Value".to_owned()),
                display_name: Some(en_us("The Key")),
                display_value: Some(en_us("The Value")),
            }],
        };
        let mut out = Vec::new();
        encode_dict(&dict, &mut out);
        assert_eq!(&out[12..16], &32u32.to_be_bytes());
        assert_eq!(decode_dict(&out).unwrap(), dict);
    }

    #[test]
    fn display_name_only_uses_24_byte_records() {
        let dict = Dict {
            entries: vec![DictEntry {
                name: "Key".to_owned(),
                value: None,
                display_name: Some(en_us("The Key")),
                display_value: None,
            }],
        };
        let mut out = Vec::new();
        encode_dict(&dict, &mut out);
        assert_eq!(&out[12..16], &24u32.to_be_bytes());
        assert_eq!(decode_dict(&out).unwrap(), dict);
    }

    #[test]
    fn rejects_invalid_record_size() {
        let mut e = b"dict\x00\x00\x00\x00".to_vec();
        e.extend_from_slice(&1u32.to_be_bytes()); // one record
        e.extend_from_slice(&20u32.to_be_bytes()); // record size 20 — not 16/24/32
        e.extend_from_slice(&[0u8; 20]);
        assert!(decode_dict(&e).is_err());
    }
}
