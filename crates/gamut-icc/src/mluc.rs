//! Language-tagged text elements: `multiLocalizedUnicodeType` (v4) and the legacy
//! `textDescriptionType` (v2) (ICC.1:2022 §10.13; ICC.1:2001-04 §6.5.17).

use gamut_core::{Error, Result};

use crate::bytes::ByteReader;

/// One localized string in a [`Mluc`]: an ISO 639 language code, an ISO 3166 country code, and the
/// text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MlucRecord {
    /// Two-letter ISO 639-1 language code (e.g. `b"en"`).
    pub language: [u8; 2],
    /// Two-letter ISO 3166-1 country code (e.g. `b"US"`).
    pub country: [u8; 2],
    /// The localized text.
    pub text: String,
}

/// A `multiLocalizedUnicodeType` element (ICC.1:2022 §10.13): one or more localized strings, the v4
/// representation of `desc`, `cprt`, and similar text tags.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Mluc {
    /// The localized strings, in file order.
    pub records: Vec<MlucRecord>,
}

impl Mluc {
    /// The text for an exact language/country match, if present.
    #[must_use]
    pub fn text(&self, language: &[u8; 2], country: &[u8; 2]) -> Option<&str> {
        self.records
            .iter()
            .find(|r| &r.language == language && &r.country == country)
            .map(|r| r.text.as_str())
    }

    /// The first record's text, if any — a reasonable default when no specific locale is wanted.
    #[must_use]
    pub fn first(&self) -> Option<&str> {
        self.records.first().map(|r| r.text.as_str())
    }
}

/// A `textDescriptionType` element (ICC.1:2001-04 §6.5.17): the v2 representation of the `desc` tag.
///
/// Carries the same description in up to three forms; the 7-bit ASCII form is the one universally
/// populated and the [`ascii`](Self::ascii) field is what most callers want. The Unicode and
/// Macintosh ScriptCode forms are decoded too so the element round-trips.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextDescription {
    /// The 7-bit ASCII description.
    pub ascii: String,
    /// The language code for the Unicode form.
    pub unicode_language: u32,
    /// The Unicode description (empty when absent).
    pub unicode: String,
    /// The Macintosh ScriptCode code for the Macintosh form.
    pub script_code: u16,
    /// The Macintosh ScriptCode bytes (Mac OS Roman, at most 67; empty when absent). Kept as raw
    /// bytes because the obsolete Mac OS Roman encoding is not otherwise modelled.
    pub macintosh: Vec<u8>,
}

/// Decodes a `multiLocalizedUnicodeType` element.
pub(crate) fn decode_mluc(element: &[u8]) -> Result<Mluc> {
    let mut r = ByteReader::at(element, 8)?;
    let count = r.u32()? as usize;
    let record_size = r.u32()? as usize;
    if record_size < 12 {
        return Err(Error::InvalidInput("icc: mluc record size too small"));
    }
    // Bound the record table against the element before allocating.
    let table_fits = count
        .checked_mul(record_size)
        .and_then(|n| n.checked_add(16))
        .is_some_and(|end| end <= element.len());
    if !table_fits {
        return Err(Error::InvalidInput(
            "icc: mluc record table exceeds element",
        ));
    }

    // Pass 1: read the record metadata (the string storage is resolved by offset afterwards).
    let mut metas = Vec::with_capacity(count);
    for _ in 0..count {
        let language = r.u16()?.to_be_bytes();
        let country = r.u16()?.to_be_bytes();
        let length = r.u32()? as usize;
        let offset = r.u32()? as usize;
        r.skip(record_size - 12)?; // skip any extra bytes a future record layout adds
        metas.push((language, country, length, offset));
    }

    // Pass 2: resolve each string from its offset (relative to the element start).
    let mut records = Vec::with_capacity(count);
    for (language, country, length, offset) in metas {
        let end = offset
            .checked_add(length)
            .ok_or(Error::InvalidInput("icc: mluc string overflow"))?;
        let bytes = element
            .get(offset..end)
            .ok_or(Error::InvalidInput("icc: mluc string out of bounds"))?;
        records.push(MlucRecord {
            language,
            country,
            text: decode_utf16be(bytes)?,
        });
    }
    Ok(Mluc { records })
}

/// Decodes a `textDescriptionType` element.
pub(crate) fn decode_text_description(element: &[u8]) -> Result<TextDescription> {
    let mut r = ByteReader::at(element, 8)?;

    let ascii_count = r.u32()? as usize;
    let ascii_bytes = r.bytes(ascii_count)?;
    if !ascii_bytes.is_ascii() {
        return Err(Error::InvalidInput("icc: non-ASCII textDescription"));
    }
    let end = ascii_bytes
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(ascii_count);
    let ascii: String = ascii_bytes[..end].iter().map(|&b| b as char).collect();

    let unicode_language = r.u32()?;
    let unicode_count = r.u32()? as usize; // number of UTF-16 code units, including the NUL
    let unicode_byte_len = unicode_count
        .checked_mul(2)
        .ok_or(Error::InvalidInput("icc: textDescription unicode overflow"))?;
    let unicode = decode_utf16be(r.bytes(unicode_byte_len)?)?;

    let script_code = r.u16()?;
    let mac_count = r.u8()? as usize;
    let mac_buffer = r.bytes(67)?;
    if mac_count > mac_buffer.len() {
        return Err(Error::InvalidInput(
            "icc: textDescription ScriptCode count too large",
        ));
    }
    let macintosh = mac_buffer[..mac_count].to_vec();

    Ok(TextDescription {
        ascii,
        unicode_language,
        unicode,
        script_code,
        macintosh,
    })
}

/// The serialized byte length of the `multiLocalizedUnicodeType` element at the start of `bytes`.
///
/// `profileSequenceDescType` embeds these descriptions back-to-back with no length prefix, so the
/// end of one must be computed from its own record table: the furthest of the table end and every
/// record's `offset + length` (offsets are relative to the element start; strings follow the table).
pub(crate) fn mluc_len(bytes: &[u8]) -> Result<usize> {
    let mut r = ByteReader::at(bytes, 8)?;
    let count = r.u32()? as usize;
    let record_size = r.u32()? as usize;
    if record_size < 12 {
        return Err(Error::InvalidInput("icc: mluc record size too small"));
    }
    let table_end = count
        .checked_mul(record_size)
        .and_then(|n| n.checked_add(16))
        .ok_or(Error::InvalidInput("icc: mluc record table overflow"))?;
    let mut end = table_end;
    for i in 0..count {
        // Each record: language(2) + country(2) + length(4) + offset(4); read the last two.
        let mut rr = ByteReader::at(bytes, 16 + i * record_size + 4)?;
        let length = rr.u32()? as usize;
        let offset = rr.u32()? as usize;
        let extent = offset
            .checked_add(length)
            .ok_or(Error::InvalidInput("icc: mluc string overflow"))?;
        end = end.max(extent);
    }
    Ok(end)
}

/// The serialized byte length of the `textDescriptionType` element at the start of `bytes` (the v2
/// counterpart of [`mluc_len`], for descriptions embedded in `profileSequenceDescType`).
pub(crate) fn text_description_len(bytes: &[u8]) -> Result<usize> {
    let ascii_count = ByteReader::at(bytes, 8)?.u32()? as usize;
    // The Unicode count sits after the ASCII block and the 4-byte Unicode language code.
    let unicode_count = ByteReader::at(bytes, 16 + ascii_count)?.u32()? as usize;
    // 8 (header) + 4 + ascii + 4 + 4 + 2·unicode + 2 (script) + 1 (mac count) + 67 (mac buffer).
    let unicode_bytes = unicode_count
        .checked_mul(2)
        .ok_or(Error::InvalidInput("icc: textDescription unicode overflow"))?;
    90usize
        .checked_add(ascii_count)
        .and_then(|n| n.checked_add(unicode_bytes))
        .ok_or(Error::InvalidInput("icc: textDescription length overflow"))
}

/// Writes a `multiLocalizedUnicodeType` element — the inverse of [`decode_mluc`]. Strings are laid
/// out after the 12-byte record table; offsets are recomputed (so they need not match the input's).
pub(crate) fn encode_mluc(mluc: &Mluc, out: &mut Vec<u8>) {
    out.extend_from_slice(b"mluc");
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(mluc.records.len() as u32).to_be_bytes());
    out.extend_from_slice(&12u32.to_be_bytes()); // record size

    let storage_start = 16 + mluc.records.len() * 12;
    let mut storage = Vec::new();
    let mut table = Vec::new();
    for record in &mluc.records {
        let utf16: Vec<u8> = record
            .text
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect();
        let offset = (storage_start + storage.len()) as u32;
        table.push((record.language, record.country, utf16.len() as u32, offset));
        storage.extend_from_slice(&utf16);
    }
    for (language, country, length, offset) in table {
        out.extend_from_slice(&language);
        out.extend_from_slice(&country);
        out.extend_from_slice(&length.to_be_bytes());
        out.extend_from_slice(&offset.to_be_bytes());
    }
    out.extend_from_slice(&storage);
}

/// Writes a `textDescriptionType` element — the inverse of [`decode_text_description`].
pub(crate) fn encode_text_description(desc: &TextDescription, out: &mut Vec<u8>) {
    out.extend_from_slice(b"desc");
    out.extend_from_slice(&[0; 4]);

    let ascii = desc.ascii.as_bytes();
    out.extend_from_slice(&((ascii.len() + 1) as u32).to_be_bytes());
    out.extend_from_slice(ascii);
    out.push(0); // ASCII NUL terminator

    out.extend_from_slice(&desc.unicode_language.to_be_bytes());
    if desc.unicode.is_empty() {
        out.extend_from_slice(&0u32.to_be_bytes());
    } else {
        let utf16: Vec<u8> = desc
            .unicode
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect();
        out.extend_from_slice(&((utf16.len() / 2 + 1) as u32).to_be_bytes()); // units incl NUL
        out.extend_from_slice(&utf16);
        out.extend_from_slice(&[0, 0]); // UTF-16 NUL terminator
    }

    out.extend_from_slice(&desc.script_code.to_be_bytes());
    out.push(desc.macintosh.len().min(67) as u8);
    let mut mac = [0u8; 67];
    let n = desc.macintosh.len().min(67);
    mac[..n].copy_from_slice(&desc.macintosh[..n]);
    out.extend_from_slice(&mac);
}

/// Decodes UTF-16BE bytes into a `String`, trimming any trailing NUL terminators.
fn decode_utf16be(bytes: &[u8]) -> Result<String> {
    if !bytes.len().is_multiple_of(2) {
        return Err(Error::InvalidInput("icc: odd-length UTF-16 string"));
    }
    let units = bytes
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]));
    let mut text = String::new();
    for unit in char::decode_utf16(units) {
        text.push(unit.map_err(|_| Error::InvalidInput("icc: invalid UTF-16 string"))?);
    }
    while text.ends_with('\0') {
        text.pop();
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16be(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(u16::to_be_bytes).collect()
    }

    #[test]
    fn decodes_mluc_records() {
        let mut e = b"mluc\x00\x00\x00\x00".to_vec();
        e.extend_from_slice(&2u32.to_be_bytes()); // record count
        e.extend_from_slice(&12u32.to_be_bytes()); // record size
        let storage = 8 + 8 + 2 * 12; // header + count/size + two 12-byte records
        let hi = utf16be("Hi");
        let hallo = utf16be("Hallo");
        // record 0: en/US
        e.extend_from_slice(b"enUS");
        e.extend_from_slice(&(hi.len() as u32).to_be_bytes());
        e.extend_from_slice(&(storage as u32).to_be_bytes());
        // record 1: de/DE
        e.extend_from_slice(b"deDE");
        e.extend_from_slice(&(hallo.len() as u32).to_be_bytes());
        e.extend_from_slice(&((storage + hi.len()) as u32).to_be_bytes());
        e.extend_from_slice(&hi);
        e.extend_from_slice(&hallo);

        let mluc = decode_mluc(&e).unwrap();
        assert_eq!(mluc.records.len(), 2);
        assert_eq!(mluc.text(b"en", b"US"), Some("Hi"));
        assert_eq!(mluc.text(b"de", b"DE"), Some("Hallo"));
        assert_eq!(mluc.text(b"fr", b"FR"), None);
        // A mixed query (one field from each record) must match neither — the language *and* the
        // country have to agree.
        assert_eq!(mluc.text(b"en", b"DE"), None);
        assert_eq!(mluc.first(), Some("Hi"));
    }

    #[test]
    fn rejects_mluc_with_oversized_table() {
        let mut e = b"mluc\x00\x00\x00\x00".to_vec();
        e.extend_from_slice(&0xFFFF_FFFFu32.to_be_bytes()); // absurd count
        e.extend_from_slice(&12u32.to_be_bytes());
        assert!(decode_mluc(&e).is_err());
    }

    #[test]
    fn decodes_text_description_ascii_only() {
        let mut e = b"desc\x00\x00\x00\x00".to_vec();
        e.extend_from_slice(&5u32.to_be_bytes()); // ASCII count (incl NUL)
        e.extend_from_slice(b"Test\0");
        e.extend_from_slice(&0u32.to_be_bytes()); // unicode language
        e.extend_from_slice(&0u32.to_be_bytes()); // unicode count
        e.extend_from_slice(&0u16.to_be_bytes()); // script code
        e.push(0); // mac count
        e.extend_from_slice(&[0u8; 67]); // mac buffer

        let desc = decode_text_description(&e).unwrap();
        assert_eq!(desc.ascii, "Test");
        assert!(desc.unicode.is_empty());
        assert!(desc.macintosh.is_empty());
    }

    #[test]
    fn decodes_text_description_with_unicode() {
        let mut e = b"desc\x00\x00\x00\x00".to_vec();
        e.extend_from_slice(&2u32.to_be_bytes());
        e.extend_from_slice(b"A\0");
        e.extend_from_slice(&0u32.to_be_bytes()); // unicode language
        let unicode = utf16be("B\0"); // includes the NUL terminator
        e.extend_from_slice(&((unicode.len() / 2) as u32).to_be_bytes());
        e.extend_from_slice(&unicode);
        e.extend_from_slice(&0u16.to_be_bytes());
        e.push(0);
        e.extend_from_slice(&[0u8; 67]);

        let desc = decode_text_description(&e).unwrap();
        assert_eq!(desc.ascii, "A");
        assert_eq!(desc.unicode, "B");
    }

    #[test]
    fn mluc_round_trips_through_encode() {
        let mluc = Mluc {
            records: vec![
                MlucRecord {
                    language: *b"en",
                    country: *b"US",
                    text: "Hello".to_owned(),
                },
                MlucRecord {
                    language: *b"de",
                    country: *b"DE",
                    text: "Hallo".to_owned(),
                },
            ],
        };
        let mut out = Vec::new();
        encode_mluc(&mluc, &mut out);
        assert_eq!(decode_mluc(&out).unwrap(), mluc);
    }

    #[test]
    fn rejects_mluc_with_small_record_size() {
        let mut e = b"mluc\x00\x00\x00\x00".to_vec();
        e.extend_from_slice(&1u32.to_be_bytes()); // one record
        e.extend_from_slice(&8u32.to_be_bytes()); // record size 8 (< the 12-byte minimum)
        e.extend_from_slice(&[0u8; 12]); // room for the mutant to read a record before failing
        assert!(decode_mluc(&e).is_err());
    }

    #[test]
    fn text_description_accepts_maximum_macintosh_count() {
        let mut e = b"desc\x00\x00\x00\x00".to_vec();
        e.extend_from_slice(&1u32.to_be_bytes()); // ASCII count (just the NUL)
        e.push(0);
        e.extend_from_slice(&0u32.to_be_bytes()); // unicode language
        e.extend_from_slice(&0u32.to_be_bytes()); // unicode count
        e.extend_from_slice(&0u16.to_be_bytes()); // script code
        e.push(67); // the maximum ScriptCode count
        e.extend_from_slice(&[5u8; 67]);
        let desc = decode_text_description(&e).unwrap();
        assert_eq!(desc.macintosh.len(), 67);
    }

    #[test]
    fn mluc_len_matches_encoded_length_even_with_trailing_bytes() {
        let mluc = Mluc {
            records: vec![MlucRecord {
                language: *b"en",
                country: *b"US",
                text: "Hello".to_owned(),
            }],
        };
        let mut bytes = Vec::new();
        encode_mluc(&mluc, &mut bytes);
        let encoded_len = bytes.len();
        bytes.extend_from_slice(b"TRAILING DATA"); // as if another element followed
        assert_eq!(mluc_len(&bytes).unwrap(), encoded_len);
    }

    #[test]
    fn text_description_len_matches_encoded_length() {
        let desc = TextDescription {
            ascii: "Model".to_owned(),
            unicode_language: 0,
            unicode: "Model".to_owned(),
            script_code: 0,
            macintosh: Vec::new(),
        };
        let mut bytes = Vec::new();
        encode_text_description(&desc, &mut bytes);
        let encoded_len = bytes.len();
        bytes.extend_from_slice(&[0xAB; 8]);
        assert_eq!(text_description_len(&bytes).unwrap(), encoded_len);
    }

    #[test]
    fn text_description_round_trips_through_encode() {
        // Exercises the Unicode-present and Macintosh-present branches of the encoder.
        let desc = TextDescription {
            ascii: "Display".to_owned(),
            unicode_language: 0x656e_5553,
            unicode: "Display".to_owned(),
            script_code: 0,
            macintosh: vec![1, 2, 3],
        };
        let mut out = Vec::new();
        encode_text_description(&desc, &mut out);
        assert_eq!(decode_text_description(&out).unwrap(), desc);
    }
}
