//! Parsing of the TIFF byte-order header and the IFD chain.

use gamut_core::{Error, Result};

use crate::ifd::{ByteOrder, FieldType, Ifd, Value};

/// A parsed TIFF file: its byte order and the chain of Image File Directories.
#[derive(Debug, Clone, PartialEq)]
pub struct TiffFile {
    /// The byte order the file was written in.
    pub order: ByteOrder,
    /// The Image File Directories, in file order (one per subfile/page).
    pub ifds: Vec<Ifd>,
}

/// The fixed size of one IFD entry on disk, in bytes (tag + type + count + value/offset).
const ENTRY_SIZE: usize = 12;
/// An upper bound on the number of IFDs followed, to bound malformed/looping chains.
const MAX_IFDS: usize = 1 << 16;

/// Reads a 16-bit value at `pos` in `order`, bounds-checked.
fn u16_at(data: &[u8], pos: usize, order: ByteOrder) -> Result<u16> {
    let b = data
        .get(pos..pos + 2)
        .ok_or(Error::InvalidInput("TIFF: truncated 16-bit field"))?;
    Ok(order.u16([b[0], b[1]]))
}

/// Reads a 32-bit value at `pos` in `order`, bounds-checked.
fn u32_at(data: &[u8], pos: usize, order: ByteOrder) -> Result<u32> {
    let b = data
        .get(pos..pos + 4)
        .ok_or(Error::InvalidInput("TIFF: truncated 32-bit field"))?;
    Ok(order.u32([b[0], b[1], b[2], b[3]]))
}

/// Parses the 8-byte image file header, returning the byte order and the offset of the first IFD.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if the byte-order mark or magic number is not valid.
pub fn read_header(data: &[u8]) -> Result<(ByteOrder, u32)> {
    let head = data
        .get(..8)
        .ok_or(Error::InvalidInput("TIFF: header too short"))?;
    let order = match [head[0], head[1]] {
        [0x49, 0x49] => ByteOrder::LittleEndian,
        [0x4D, 0x4D] => ByteOrder::BigEndian,
        _ => return Err(Error::InvalidInput("TIFF: bad byte-order mark")),
    };
    if order.u16([head[2], head[3]]) != 42 {
        return Err(Error::InvalidInput("TIFF: bad magic number"));
    }
    Ok((order, order.u32([head[4], head[5], head[6], head[7]])))
}

/// Reads the single IFD at `offset`, returning it and the offset of the next IFD (`0` if last).
fn read_ifd(data: &[u8], offset: usize, order: ByteOrder) -> Result<(Ifd, usize)> {
    let count = u16_at(data, offset, order)? as usize;
    let entries_start = offset + 2;
    let next_pos = entries_start + count * ENTRY_SIZE;
    // Bound the directory to the file so a corrupt count fails fast rather than allocating.
    if next_pos + 4 > data.len() {
        return Err(Error::InvalidInput("TIFF: IFD extends past end of file"));
    }
    let mut ifd = Ifd::new();
    for i in 0..count {
        let pos = entries_start + i * ENTRY_SIZE;
        let tag = u16_at(data, pos, order)?;
        let type_code = u16_at(data, pos + 2, order)?;
        let value_count = u32_at(data, pos + 4, order)? as usize;
        // Per spec, readers skip fields with an unexpected (unknown) field type.
        let Some(ty) = FieldType::from_code(type_code) else {
            continue;
        };
        let byte_len = value_count
            .checked_mul(ty.size())
            .ok_or(Error::InvalidInput("TIFF: field length overflow"))?;
        let value = if byte_len <= 4 {
            Value::decode(ty, value_count, &data[pos + 8..pos + 12], order)?
        } else {
            let voff = u32_at(data, pos + 8, order)? as usize;
            let bytes = data
                .get(voff..)
                .ok_or(Error::InvalidInput("TIFF: value offset out of bounds"))?;
            Value::decode(ty, value_count, bytes, order)?
        };
        // A duplicate tag keeps the last occurrence; `set` maintains sort order.
        ifd.set(tag, value);
    }
    let next = u32_at(data, next_pos, order)? as usize;
    Ok((ifd, next))
}

/// Parses a TIFF file: the header followed by the whole IFD chain.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if the header is invalid, an offset is out of bounds, the IFD
/// chain loops, or a field value is truncated.
pub fn read(data: &[u8]) -> Result<TiffFile> {
    let (order, first) = read_header(data)?;
    let mut ifds = Vec::new();
    let mut offset = first as usize;
    let mut seen = Vec::new();
    while offset != 0 {
        if seen.contains(&offset) {
            return Err(Error::InvalidInput("TIFF: IFD chain loops"));
        }
        if ifds.len() >= MAX_IFDS {
            return Err(Error::InvalidInput("TIFF: too many IFDs"));
        }
        seen.push(offset);
        let (ifd, next) = read_ifd(data, offset, order)?;
        ifds.push(ifd);
        offset = next;
    }
    if ifds.is_empty() {
        return Err(Error::InvalidInput("TIFF: file has no IFD"));
    }
    Ok(TiffFile { order, ifds })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_header() {
        assert!(read_header(b"\x49\x49").is_err()); // too short
        assert!(read_header(b"XX\x2a\x00\x08\x00\x00\x00").is_err()); // bad BOM
        assert!(read_header(b"II\x00\x00\x08\x00\x00\x00").is_err()); // bad magic
        let (order, first) = read_header(b"II\x2a\x00\x08\x00\x00\x00").expect("ok");
        assert_eq!(order, ByteOrder::LittleEndian);
        assert_eq!(first, 8);
    }

    #[test]
    fn empty_input_errors() {
        assert!(read(&[]).is_err());
    }
}
