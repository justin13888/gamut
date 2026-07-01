//! Colorant elements: `colorantOrderType` (ICC.1:2022 §10.4) and `colorantTableType` (§10.5).

use gamut_core::{Error, Result};

use crate::bytes::ByteReader;

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

/// Serializes a `colorantTableType` element (the inverse of [`decode_colorant_table`]).
pub(crate) fn encode_colorant_table(clrt: &ColorantTable, out: &mut Vec<u8>) {
    out.extend_from_slice(b"clrt");
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(clrt.colorants.len() as u32).to_be_bytes());
    for colorant in &clrt.colorants {
        let mut field = [0u8; COLORANT_NAME_LEN];
        let name = colorant.name.as_bytes();
        let n = name.len().min(COLORANT_NAME_LEN);
        field[..n].copy_from_slice(&name[..n]);
        out.extend_from_slice(&field);
        for component in colorant.pcs {
            out.extend_from_slice(&component.to_be_bytes());
        }
    }
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
        encode_colorant_table(&clrt, &mut out);
        assert_eq!(decode_colorant_table(&out).unwrap(), clrt);
    }

    #[test]
    fn rejects_colorant_table_that_overflows_element() {
        let payload = 0xFFFF_FFFFu32.to_be_bytes().to_vec(); // absurd count, no data
        assert!(decode_colorant_table(&element(b"clrt", &payload)).is_err());
    }
}
