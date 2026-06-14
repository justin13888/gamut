//! The named-colour element type `namedColor2Type` (ICC.1:2022 §10.17).

use gamut_core::{Error, Result};

use crate::bytes::ByteReader;

/// One entry of a [`NamedColor2`]: a colour's root name plus its PCS and device coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedColor {
    /// The colour's root name (without the list's shared prefix/suffix).
    pub name: String,
    /// The PCS coordinates (the encoding — XYZ or Lab as `u16` — follows the profile's PCS).
    pub pcs: [u16; 3],
    /// The device coordinates, one per device channel.
    pub device: Vec<u16>,
}

/// A `namedColor2Type` element (ICC.1:2022 §10.17): a named-colour palette with a shared
/// prefix/suffix and a per-colour name plus PCS and device coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedColor2 {
    /// Vendor-specific flags (bytes 8–11 of the element).
    pub vendor_flags: u32,
    /// The prefix applied to every colour name.
    pub prefix: String,
    /// The suffix applied to every colour name.
    pub suffix: String,
    /// The named colours, in order.
    pub colors: Vec<NamedColor>,
}

/// Decodes a `namedColor2Type` element.
pub(crate) fn decode_named_color2(element: &[u8]) -> Result<NamedColor2> {
    let mut r = ByteReader::at(element, 8)?;
    let vendor_flags = r.u32()?;
    let count = r.u32()? as usize;
    let device_coords = r.u32()? as usize;
    let prefix = read_ascii_32(&mut r)?;
    let suffix = read_ascii_32(&mut r)?;

    // Bound the colour records against the element before allocating.
    let entry_size = device_coords
        .checked_mul(2)
        .and_then(|n| n.checked_add(38)) // 32-byte name + three PCS u16
        .ok_or(Error::InvalidInput("icc: named-colour entry overflow"))?;
    let records_fit = count
        .checked_mul(entry_size)
        .is_some_and(|n| n <= r.remaining());
    if !records_fit {
        return Err(Error::InvalidInput(
            "icc: named-colour list exceeds element",
        ));
    }

    let mut colors = Vec::with_capacity(count);
    for _ in 0..count {
        let name = read_ascii_32(&mut r)?;
        let pcs = [r.u16()?, r.u16()?, r.u16()?];
        let mut device = Vec::with_capacity(device_coords);
        for _ in 0..device_coords {
            device.push(r.u16()?);
        }
        colors.push(NamedColor { name, pcs, device });
    }
    Ok(NamedColor2 {
        vendor_flags,
        prefix,
        suffix,
        colors,
    })
}

/// Writes a `namedColor2Type` element — the inverse of [`decode_named_color2`].
pub(crate) fn encode_named_color2(named: &NamedColor2, out: &mut Vec<u8>) {
    let device_coords = named.colors.first().map_or(0, |c| c.device.len());
    out.extend_from_slice(b"ncl2");
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&named.vendor_flags.to_be_bytes());
    out.extend_from_slice(&(named.colors.len() as u32).to_be_bytes());
    out.extend_from_slice(&(device_coords as u32).to_be_bytes());
    write_ascii_32(out, &named.prefix);
    write_ascii_32(out, &named.suffix);
    for color in &named.colors {
        write_ascii_32(out, &color.name);
        for &p in &color.pcs {
            out.extend_from_slice(&p.to_be_bytes());
        }
        for &d in &color.device {
            out.extend_from_slice(&d.to_be_bytes());
        }
    }
}

/// Writes a 32-byte NUL-terminated 7-bit-ASCII field (truncating to leave room for the NUL).
fn write_ascii_32(out: &mut Vec<u8>, text: &str) {
    let mut field = [0u8; 32];
    let n = text.len().min(31);
    field[..n].copy_from_slice(&text.as_bytes()[..n]);
    out.extend_from_slice(&field);
}

/// Reads a 32-byte NUL-terminated 7-bit-ASCII field.
fn read_ascii_32(r: &mut ByteReader<'_>) -> Result<String> {
    let bytes = r.bytes(32)?;
    if !bytes.is_ascii() {
        return Err(Error::InvalidInput("icc: non-ASCII named-colour string"));
    }
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    Ok(bytes[..end].iter().map(|&b| b as char).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ascii_32(s: &str) -> Vec<u8> {
        let mut buf = vec![0u8; 32];
        buf[..s.len()].copy_from_slice(s.as_bytes());
        buf
    }

    #[test]
    fn decodes_named_colors() {
        let mut e = b"ncl2\x00\x00\x00\x00".to_vec();
        e.extend_from_slice(&0u32.to_be_bytes()); // vendor flags
        e.extend_from_slice(&1u32.to_be_bytes()); // one colour
        e.extend_from_slice(&2u32.to_be_bytes()); // two device coordinates
        e.extend_from_slice(&ascii_32("pre")); // prefix
        e.extend_from_slice(&ascii_32("suf")); // suffix
        e.extend_from_slice(&ascii_32("red")); // colour name
        for v in [1u16, 2, 3] {
            e.extend_from_slice(&v.to_be_bytes()); // PCS
        }
        for v in [10u16, 20] {
            e.extend_from_slice(&v.to_be_bytes()); // device
        }

        let named = decode_named_color2(&e).unwrap();
        assert_eq!(named.prefix, "pre");
        assert_eq!(named.suffix, "suf");
        assert_eq!(named.colors.len(), 1);
        assert_eq!(named.colors[0].name, "red");
        assert_eq!(named.colors[0].pcs, [1, 2, 3]);
        assert_eq!(named.colors[0].device, vec![10, 20]);
    }

    #[test]
    fn rejects_oversized_list() {
        let mut e = b"ncl2\x00\x00\x00\x00".to_vec();
        e.extend_from_slice(&0u32.to_be_bytes());
        e.extend_from_slice(&0xFFFF_FFFFu32.to_be_bytes()); // absurd colour count
        e.extend_from_slice(&3u32.to_be_bytes());
        e.extend_from_slice(&ascii_32("")); // prefix
        e.extend_from_slice(&ascii_32("")); // suffix
        assert!(decode_named_color2(&e).is_err());
    }

    #[test]
    fn round_trips_through_encode() {
        let named = NamedColor2 {
            vendor_flags: 0x1234,
            prefix: "pre".into(),
            suffix: "suf".into(),
            colors: vec![
                NamedColor {
                    name: "red".into(),
                    pcs: [1, 2, 3],
                    device: vec![10, 20],
                },
                NamedColor {
                    name: "green".into(),
                    pcs: [4, 5, 6],
                    device: vec![30, 40],
                },
            ],
        };
        let mut out = Vec::new();
        encode_named_color2(&named, &mut out);
        assert_eq!(decode_named_color2(&out).unwrap(), named);
    }
}
