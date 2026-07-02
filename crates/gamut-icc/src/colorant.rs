//! Colorant elements: `colorantOrderType` (ICC.1:2022 §10.4) and `colorantTableType` (§10.5).

use gamut_core::{Error, Result};

use crate::bytes::{ByteReader, push_ascii_32};

/// The 32-byte fixed field a colorant name occupies (NUL-terminated 7-bit ASCII, §10.5 Table 34).
const COLORANT_NAME_LEN: usize = 32;

/// A `colorantOrderType` element (§10.4): the laydown order of an n-colorant device's colorants,
/// each entry the number of the colorant printed in that position (first laid down at index 0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorantOrder {
    /// The colorant numbers in laydown order; its length is the colorant count.
    pub order: Vec<u8>,
}

/// Decodes a `colorantOrderType` element.
pub(crate) fn decode_colorant_order(element: &[u8]) -> Result<ColorantOrder> {
    let mut r = ByteReader::at(element, 8)?;
    let count = r.u32()? as usize;
    let order = r.bytes(count)?.to_vec();
    Ok(ColorantOrder { order })
}

/// Serializes a `colorantOrderType` element (the inverse of [`decode_colorant_order`]).
pub(crate) fn encode_colorant_order(clro: &ColorantOrder, out: &mut Vec<u8>) {
    out.extend_from_slice(b"clro");
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(clro.order.len() as u32).to_be_bytes());
    out.extend_from_slice(&clro.order);
}

/// One colorant of a [`ColorantTable`]: a name and its PCS coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Colorant {
    /// The colorant name (7-bit ASCII, up to 31 characters).
    pub name: String,
    /// The colorant's PCS value (three `uInt16` in the profile's PCS, relative colorimetric).
    pub pcs: [u16; 3],
}

/// A `colorantTableType` element (§10.5): names and PCS values for each device-channel colorant, in
/// channel order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorantTable {
    /// The colorants, one per device channel, in channel order.
    pub colorants: Vec<Colorant>,
}

/// Decodes a `colorantTableType` element.
pub(crate) fn decode_colorant_table(element: &[u8]) -> Result<ColorantTable> {
    let mut r = ByteReader::at(element, 8)?;
    let count = r.u32()? as usize;
    // Each colorant is a 32-byte name + three uInt16; bound the total before allocating.
    if count
        .checked_mul(COLORANT_NAME_LEN + 6)
        .is_none_or(|n| n > r.remaining())
    {
        return Err(Error::InvalidInput("icc: colorant table exceeds element"));
    }
    let mut colorants = Vec::with_capacity(count);
    for _ in 0..count {
        let name = decode_colorant_name(r.bytes(COLORANT_NAME_LEN)?)?;
        let pcs = [r.u16()?, r.u16()?, r.u16()?];
        colorants.push(Colorant { name, pcs });
    }
    Ok(ColorantTable { colorants })
}

/// Reads a colorant name from its 32-byte NUL-terminated ASCII field.
fn decode_colorant_name(field: &[u8]) -> Result<String> {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    let used = &field[..end];
    if !used.is_ascii() {
        return Err(Error::InvalidInput("icc: non-ASCII colorant name"));
    }
    Ok(used.iter().map(|&b| b as char).collect())
}

/// Serializes a `colorantTableType` element (the inverse of [`decode_colorant_table`]); colorant
/// names are validated against their 32-byte ASCII fields rather than truncated.
pub(crate) fn encode_colorant_table(clrt: &ColorantTable, out: &mut Vec<u8>) -> Result<()> {
    out.extend_from_slice(b"clrt");
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(clrt.colorants.len() as u32).to_be_bytes());
    for colorant in &clrt.colorants {
        push_ascii_32(out, &colorant.name)?;
        for component in colorant.pcs {
            out.extend_from_slice(&component.to_be_bytes());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(type_sig: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut e = Vec::with_capacity(8 + payload.len());
        e.extend_from_slice(type_sig);
        e.extend_from_slice(&[0; 4]);
        e.extend_from_slice(payload);
        e
    }

    #[test]
    fn decodes_colorant_order() {
        let mut payload = 4u32.to_be_bytes().to_vec();
        payload.extend_from_slice(&[3, 0, 1, 2]); // KCMY-style laydown
        let clro = decode_colorant_order(&element(b"clro", &payload)).unwrap();
        assert_eq!(clro.order, vec![3, 0, 1, 2]);
    }

    #[test]
    fn colorant_order_round_trips_through_encode() {
        let clro = ColorantOrder {
            order: vec![0, 1, 2],
        };
        let mut out = Vec::new();
        encode_colorant_order(&clro, &mut out);
        assert_eq!(decode_colorant_order(&out).unwrap(), clro);
    }

    #[test]
    fn decodes_colorant_table_name_and_pcs() {
        let mut payload = 1u32.to_be_bytes().to_vec();
        let mut name = [0u8; 32];
        name[..4].copy_from_slice(b"Cyan");
        payload.extend_from_slice(&name);
        for v in [10u16, 20, 30] {
            payload.extend_from_slice(&v.to_be_bytes());
        }
        let clrt = decode_colorant_table(&element(b"clrt", &payload)).unwrap();
        assert_eq!(clrt.colorants.len(), 1);
        assert_eq!(clrt.colorants[0].name, "Cyan");
        assert_eq!(clrt.colorants[0].pcs, [10, 20, 30]);
    }

    #[test]
    fn colorant_table_round_trips_through_encode() {
        let clrt = ColorantTable {
            colorants: vec![
                Colorant {
                    name: "Red".to_owned(),
                    pcs: [0xFFFF, 0, 0],
                },
                Colorant {
                    name: "Green".to_owned(),
                    pcs: [0, 0xFFFF, 0],
                },
            ],
        };
        let mut out = Vec::new();
        encode_colorant_table(&clrt, &mut out).unwrap();
        assert_eq!(decode_colorant_table(&out).unwrap(), clrt);
    }

    #[test]
    fn colorant_table_decode_bounds_are_exact() {
        // Trailing bytes after the colorant records are tolerated (the guard rejects only a table
        // that *exceeds* the element).
        let mut payload = 1u32.to_be_bytes().to_vec();
        let mut name = [0u8; 32];
        name[..4].copy_from_slice(b"Cyan");
        payload.extend_from_slice(&name);
        for v in [10u16, 20, 30] {
            payload.extend_from_slice(&v.to_be_bytes());
        }
        payload.extend_from_slice(&[0xAB; 8]); // trailing slack
        assert!(decode_colorant_table(&element(b"clrt", &payload)).is_ok());

        // A truncated element is caught by the size guard itself — each colorant needs the full
        // 38 bytes (32-byte name + three u16), so the guard's message is reported, not a later
        // read error.
        let mut payload = 1u32.to_be_bytes().to_vec();
        payload.extend_from_slice(&[0u8; 30]); // 30 < 38, but more than a name-only miscount
        match decode_colorant_table(&element(b"clrt", &payload)) {
            Err(Error::InvalidInput(msg)) => {
                assert_eq!(msg, "icc: colorant table exceeds element");
            }
            other => panic!("expected the size-guard error, got {other:?}"),
        }
    }

    #[test]
    fn encode_validates_colorant_names() {
        let with_name = |name: &str| ColorantTable {
            colorants: vec![Colorant {
                name: name.to_owned(),
                pcs: [0, 0, 0],
            }],
        };
        let mut out = Vec::new();
        assert!(encode_colorant_table(&with_name(&"x".repeat(33)), &mut out).is_err());
        assert!(encode_colorant_table(&with_name("Grün"), &mut out).is_err());
        // A 32-byte name exactly fills the field and round-trips unchanged.
        let max = with_name(&"x".repeat(32));
        let mut out = Vec::new();
        encode_colorant_table(&max, &mut out).unwrap();
        assert_eq!(decode_colorant_table(&out).unwrap(), max);
    }

    #[test]
    fn rejects_colorant_table_that_overflows_element() {
        let payload = 0xFFFF_FFFFu32.to_be_bytes().to_vec(); // absurd count, no data
        assert!(decode_colorant_table(&element(b"clrt", &payload)).is_err());
    }
}
