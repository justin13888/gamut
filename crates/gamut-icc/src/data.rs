//! `dataType` (ICC.1:2022 §10.7): a container for either 7-bit ASCII or transparent binary bytes.

use crate::bytes::ByteReader;
use crate::error::Result;

/// A `dataType` element (§10.7): a flag distinguishing ASCII from binary, followed by the payload.
///
/// The payload is stored verbatim (including any ASCII `00h` terminator) so the element round-trips
/// byte-for-byte; [`DataElement::is_ascii`] interprets the flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataElement {
    /// The data flag: `0` = ASCII (`textType`) data, `1` = binary; other values are reserved.
    pub flag: u32,
    /// The raw data bytes (`element size − 12` of them).
    pub data: Vec<u8>,
}

impl DataElement {
    /// Whether the payload is 7-bit ASCII text (flag `0`); `false` for binary or reserved flags.
    #[must_use]
    pub fn is_ascii(&self) -> bool {
        self.flag == 0
    }
}

/// Decodes a `dataType` element.
pub(crate) fn decode_data(element: &[u8]) -> Result<DataElement> {
    let mut r = ByteReader::at(element, 8)?;
    let flag = r.u32()?;
    let n = r.remaining();
    let data = r.bytes(n)?.to_vec();
    Ok(DataElement { flag, data })
}

/// Serializes a `dataType` element (the inverse of [`decode_data`]).
pub(crate) fn encode_data(data: &DataElement, out: &mut Vec<u8>) {
    out.extend_from_slice(b"data");
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&data.flag.to_be_bytes());
    out.extend_from_slice(&data.data);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_ascii_payload() {
        let mut e = b"data\x00\x00\x00\x00".to_vec();
        e.extend_from_slice(&0u32.to_be_bytes()); // ASCII flag
        e.extend_from_slice(b"hi\0");
        let d = decode_data(&e).unwrap();
        assert!(d.is_ascii());
        assert_eq!(d.data, b"hi\0");
    }

    #[test]
    fn binary_flag_round_trips_through_encode() {
        let d = DataElement {
            flag: 1,
            data: vec![0x00, 0xFF, 0x80],
        };
        assert!(!d.is_ascii());
        let mut out = Vec::new();
        encode_data(&d, &mut out);
        assert_eq!(decode_data(&out).unwrap(), d);
    }
}
