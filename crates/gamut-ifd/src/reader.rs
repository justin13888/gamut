//! Parsing of the TIFF byte-order header and the IFD chain.
//!
//! The structure is offset-driven — a classic parser-exploit surface — so every access is
//! bounds-checked, the IFD chain is guarded against loops and runaway length, and unknown field
//! types are skipped rather than trusted.

use gamut_core::{Error, Result};

use crate::{ByteOrder, Coverage, FieldType, Ifd, UnknownField, Value, Variant};

/// A parsed TIFF/IFD stream: its byte order, container variant, and the chain of Image File
/// Directories.
#[derive(Debug, Clone, PartialEq)]
pub struct TiffFile {
    /// The byte order the stream was written in.
    pub order: ByteOrder,
    /// Whether the stream is classic TIFF or BigTIFF (which sizes its offsets/counts).
    pub variant: Variant,
    /// The Image File Directories, in stream order (one per subfile/page).
    pub ifds: Vec<Ifd>,
}

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

/// Reads a 64-bit value at `pos` in `order`, bounds-checked (BigTIFF offsets/counts).
#[cfg(feature = "bigtiff")]
fn u64_at(data: &[u8], pos: usize, order: ByteOrder) -> Result<u64> {
    let b = data
        .get(pos..pos + 8)
        .ok_or(Error::InvalidInput("TIFF: truncated 64-bit field"))?;
    Ok(order.u64([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
}

/// Reads an offset-sized field at `pos` (a `u32` in classic TIFF, a `u64` in BigTIFF) as `u64`.
///
/// Used for every file offset and for the per-field value count, which share the offset width.
fn offset_at(data: &[u8], pos: usize, order: ByteOrder, variant: Variant) -> Result<u64> {
    match variant {
        Variant::Classic => Ok(u64::from(u32_at(data, pos, order)?)),
        #[cfg(feature = "bigtiff")]
        Variant::Big => u64_at(data, pos, order),
    }
}

/// Parses the image file header, returning the byte order, the container variant, and the offset
/// of the first IFD. The header is 8 bytes for classic TIFF and 16 bytes for BigTIFF.
///
/// Without the `bigtiff` feature a BigTIFF magic number (`43`) is rejected as an unknown magic.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if the byte-order mark, magic number, or (for BigTIFF) the
/// fixed offset-size / reserved fields are not valid.
pub fn read_header(data: &[u8]) -> Result<(ByteOrder, Variant, u64)> {
    let head = data
        .get(..8)
        .ok_or(Error::InvalidInput("TIFF: header too short"))?;
    let order = match [head[0], head[1]] {
        [0x49, 0x49] => ByteOrder::LittleEndian,
        [0x4D, 0x4D] => ByteOrder::BigEndian,
        _ => return Err(Error::InvalidInput("TIFF: bad byte-order mark")),
    };
    match order.u16([head[2], head[3]]) {
        42 => Ok((order, Variant::Classic, u64::from(u32_at(data, 4, order)?))),
        #[cfg(feature = "bigtiff")]
        43 => {
            // BigTIFF: bytes 4-5 are the offset bytesize (always 8), bytes 6-7 are reserved (0),
            // and the first-IFD offset is the 8-byte value at bytes 8-15.
            if order.u16([head[4], head[5]]) != 8 {
                return Err(Error::InvalidInput("TIFF: BigTIFF offset size must be 8"));
            }
            if order.u16([head[6], head[7]]) != 0 {
                return Err(Error::InvalidInput(
                    "TIFF: BigTIFF reserved field must be 0",
                ));
            }
            Ok((order, Variant::Big, u64_at(data, 8, order)?))
        }
        _ => Err(Error::InvalidInput("TIFF: bad magic number")),
    }
}

/// Reads the single IFD at `offset`, returning it and the offset of the next IFD (`0` if last).
///
/// The field widths follow `variant`: the entry count is 2 bytes (classic) or 8 (BigTIFF), each
/// entry is 12 or 20 bytes, a value packs inline when it fits in the offset width (4 or 8), and
/// the next-IFD pointer is 4 or 8 bytes.
///
/// When `cov` is `Some`, the IFD body (count field + entries + next pointer) and each out-of-line
/// value's byte range are [`mark`](Coverage::mark)ed for byte-range accounting; the caller marks
/// the file header itself. When `unknown` is `Some`, an entry whose field-type code is
/// unrecognised is recorded there instead of only being skipped. With both `None` this is the
/// plain reader — no allocations, no marks — so the [`read`]/[`read_ifd_at`] hot path is unchanged.
fn read_ifd_inner(
    data: &[u8],
    offset: usize,
    order: ByteOrder,
    variant: Variant,
    mut cov: Option<&mut Coverage>,
    mut unknown: Option<&mut Vec<UnknownField>>,
) -> Result<(Ifd, u64)> {
    let entry_size = variant.entry_size();
    let inline = variant.inline_threshold();
    // The entry count is the only field whose width differs from the offset width (2 vs 8).
    let count = match variant {
        Variant::Classic => u64::from(u16_at(data, offset, order)?),
        #[cfg(feature = "bigtiff")]
        Variant::Big => u64_at(data, offset, order)?,
    } as usize;
    let entries_start = offset + variant.count_size();
    let next_pos = entries_start + count * entry_size;
    // Bound the directory to the file so a corrupt count fails fast rather than allocating.
    let body_end = next_pos + variant.offset_size();
    if body_end > data.len() {
        return Err(Error::InvalidInput("TIFF: IFD extends past end of file"));
    }
    // The directory body — count field, all entry records (inline values included), and the
    // next-IFD pointer — is one contiguous span; out-of-line values are marked per entry below.
    if let Some(c) = cov.as_deref_mut() {
        c.mark(offset as u64, (body_end - offset) as u64);
    }
    let mut ifd = Ifd::new();
    for i in 0..count {
        let pos = entries_start + i * entry_size;
        let tag = u16_at(data, pos, order)?;
        let type_code = u16_at(data, pos + 2, order)?;
        // The value count and the value/offset field both follow the offset width.
        let value_count = offset_at(data, pos + 4, order, variant)? as usize;
        let value_pos = pos + 4 + variant.offset_size();
        // Per spec, readers skip fields with an unexpected (unknown) field type; a deconstruct
        // records the skipped entry so the dropped field (and any out-of-line bytes it would have
        // covered) is surfaced rather than silently lost.
        let Some(ty) = FieldType::from_code(type_code) else {
            if let Some(u) = unknown.as_deref_mut() {
                u.push(UnknownField {
                    ifd_offset: offset as u64,
                    tag,
                    type_code,
                    count: value_count as u64,
                    entry_offset: pos as u64,
                });
            }
            continue;
        };
        let byte_len = value_count
            .checked_mul(ty.size())
            .ok_or(Error::InvalidInput("TIFF: field length overflow"))?;
        let value = if byte_len <= inline {
            Value::decode(ty, value_count, &data[value_pos..value_pos + inline], order)?
        } else {
            let voff = offset_at(data, value_pos, order, variant)? as usize;
            let bytes = data
                .get(voff..)
                .ok_or(Error::InvalidInput("TIFF: value offset out of bounds"))?;
            let value = Value::decode(ty, value_count, bytes, order)?;
            // Mark the on-disk span (`value_count * ty.size()`, padding included) — not the decoded
            // value's `byte_len`, which for ASCII stops at the first NUL and would leave the
            // padding looking like a gap.
            if let Some(c) = cov.as_deref_mut() {
                c.mark(voff as u64, byte_len as u64);
            }
            value
        };
        // A duplicate tag keeps the last occurrence; `set` maintains sort order.
        ifd.set(tag, value);
    }
    let next = offset_at(data, next_pos, order, variant)?;
    Ok((ifd, next))
}

/// Reads the single IFD located at `offset` in `data`, ignoring its next-IFD pointer.
///
/// This is how a codec follows a **sub-IFD pointer** (see [`SubIfd`](crate::SubIfd)): the generic
/// [`read`] cannot know which `LONG` tags are offsets, so it leaves a pointer tag as a plain
/// integer value; the codec reads that offset and calls this to parse the child directory (e.g. a
/// DNG raw sub-IFD via `SubIFDs`, or an `ExifIFD`). `order` and `variant` come from the enclosing
/// file's header (via [`read_header`]).
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if the directory at `offset` is out of bounds or a field value
/// is truncated.
pub fn read_ifd_at(data: &[u8], offset: u64, order: ByteOrder, variant: Variant) -> Result<Ifd> {
    read_ifd_inner(data, offset as usize, order, variant, None, None).map(|(ifd, _next)| ifd)
}

/// Like [`read_ifd_at`] but threads byte-range accounting: it marks the IFD body and every
/// out-of-line value into `cov`, records any unknown-field-type entries into `unknown`, and
/// returns the next-IFD offset alongside the directory.
///
/// A codec uses this to account a sub-IFD it reaches by following a pointer tag (a DNG raw
/// sub-IFD, `ExifIFD`, `GPSInfo`, …); the enclosing file's header and top-level chain are
/// accounted by [`read_with_coverage`].
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if the directory at `offset` is out of bounds or a field value
/// is truncated.
pub fn read_ifd_at_with_coverage(
    data: &[u8],
    offset: u64,
    order: ByteOrder,
    variant: Variant,
    cov: &mut Coverage,
    unknown: &mut Vec<UnknownField>,
) -> Result<(Ifd, u64)> {
    read_ifd_inner(
        data,
        offset as usize,
        order,
        variant,
        Some(cov),
        Some(unknown),
    )
}

/// Walks a TIFF/IFD stream's header and top-level IFD chain, optionally accounting bytes.
///
/// With both sinks `None` this is the body of [`read`]; with `Some` it additionally marks the
/// header and every IFD body/out-of-line value into `cov` and records unknown-field-type entries.
fn read_chain(
    data: &[u8],
    mut cov: Option<&mut Coverage>,
    mut unknown: Option<&mut Vec<UnknownField>>,
) -> Result<TiffFile> {
    let (order, variant, first) = read_header(data)?;
    if let Some(c) = cov.as_deref_mut() {
        c.mark(0, variant.header_size() as u64);
    }
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
        let (ifd, next) = read_ifd_inner(
            data,
            offset,
            order,
            variant,
            cov.as_deref_mut(),
            unknown.as_deref_mut(),
        )?;
        ifds.push(ifd);
        offset = next as usize;
    }
    if ifds.is_empty() {
        return Err(Error::InvalidInput("TIFF: file has no IFD"));
    }
    Ok(TiffFile {
        order,
        variant,
        ifds,
    })
}

/// Parses a TIFF/IFD stream: the header followed by the whole IFD chain.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if the header is invalid, an offset is out of bounds, the IFD
/// chain loops, or a field value is truncated.
pub fn read(data: &[u8]) -> Result<TiffFile> {
    read_chain(data, None, None)
}

/// Like [`read`] but threads byte-range accounting through the whole top-level walk: it marks the
/// header and every IFD body / out-of-line value into `cov`, and records any unknown-field-type
/// entries into `unknown`.
///
/// This accounts the header and the top-level IFD chain; a codec accounts each sub-IFD it follows
/// with [`read_ifd_at_with_coverage`] and marks its own strip/tile byte ranges, then
/// [`finish`](Coverage::finish)es `cov` into the [`CoverageReport`](crate::CoverageReport).
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] under the same conditions as [`read`].
pub fn read_with_coverage(
    data: &[u8],
    cov: &mut Coverage,
    unknown: &mut Vec<UnknownField>,
) -> Result<TiffFile> {
    read_chain(data, Some(cov), Some(unknown))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_header() {
        assert!(read_header(b"\x49\x49").is_err()); // too short
        assert!(read_header(b"XX\x2a\x00\x08\x00\x00\x00").is_err()); // bad BOM
        assert!(read_header(b"II\x00\x00\x08\x00\x00\x00").is_err()); // bad magic
        let (order, variant, first) = read_header(b"II\x2a\x00\x08\x00\x00\x00").expect("ok");
        assert_eq!(order, ByteOrder::LittleEndian);
        assert_eq!(variant, Variant::Classic);
        assert_eq!(first, 8);
    }

    #[cfg(feature = "bigtiff")]
    #[test]
    fn parses_bigtiff_header() {
        // II, magic 43, offset-size 8, reserved 0, then an 8-byte first-IFD offset of 16.
        let head = b"II\x2b\x00\x08\x00\x00\x00\x10\x00\x00\x00\x00\x00\x00\x00";
        let (order, variant, first) = read_header(head).expect("ok");
        assert_eq!(order, ByteOrder::LittleEndian);
        assert_eq!(variant, Variant::Big);
        assert_eq!(first, 16);
        // The fixed BigTIFF offset-size (8) and reserved (0) fields are validated.
        assert!(
            read_header(b"II\x2b\x00\x04\x00\x00\x00\x10\x00\x00\x00\x00\x00\x00\x00").is_err()
        );
        assert!(
            read_header(b"II\x2b\x00\x08\x00\x01\x00\x10\x00\x00\x00\x00\x00\x00\x00").is_err()
        );
        // A BigTIFF magic with a truncated (classic-length) header is rejected, not read OOB.
        assert!(read_header(b"II\x2b\x00\x08\x00\x00\x00").is_err());
    }

    /// Without the feature, a BigTIFF magic is an unknown magic, not a mis-parse.
    #[cfg(not(feature = "bigtiff"))]
    #[test]
    fn rejects_bigtiff_without_feature() {
        let head = b"II\x2b\x00\x08\x00\x00\x00\x10\x00\x00\x00\x00\x00\x00\x00";
        assert!(read_header(head).is_err());
    }

    #[test]
    fn empty_input_errors() {
        assert!(read(&[]).is_err());
    }

    #[test]
    fn rejects_truncated_ifd() {
        // Classic header with the first IFD at offset 8. The IFD declares one SHORT entry, but the
        // file ends right after the entry's count field — no room for the entry's value/offset word
        // or the next-IFD pointer. The `next_pos + offset_size > data.len()` guard must reject this;
        // without it the unchecked inline-value slice would index past the end.
        let data = [
            b'I', b'I', 0x2a, 0x00, 0x08, 0x00, 0x00, 0x00, // header: classic, first IFD @ 8
            0x01, 0x00, // entry count = 1
            0x00, 0x01, // tag 256
            0x03, 0x00, // type 3 (SHORT)
            0x01, 0x00, 0x00, 0x00, // value count = 1
        ];
        assert_eq!(data.len(), 18);
        assert!(read(&data).is_err());
    }

    /// An IFD whose out-of-line values are all even-length, so [`crate::write`] emits no alignment
    /// padding and a deconstruct can account every byte.
    fn even_value_ifd() -> Ifd {
        let mut ifd = Ifd::new();
        ifd.set(256, Value::Short(vec![640])); // inline (2 bytes)
        ifd.set(258, Value::Short(vec![8, 8, 8])); // 6 bytes -> out of line
        ifd.set(282, Value::Rational(vec![(300, 1)])); // 8 bytes -> out of line
        ifd
    }

    #[test]
    fn read_with_coverage_accounts_a_written_file() {
        let file = TiffFile {
            order: ByteOrder::LittleEndian,
            variant: Variant::Classic,
            ifds: vec![even_value_ifd()],
        };
        let bytes = crate::write(&file);
        let mut cov = Coverage::new(bytes.len() as u64);
        let mut unknown = Vec::new();
        let parsed = read_with_coverage(&bytes, &mut cov, &mut unknown).expect("read");
        // The coverage reader returns exactly what the plain reader would.
        assert_eq!(parsed, file);
        assert!(unknown.is_empty());
        let report = cov.finish();
        assert!(report.is_fully_covered(), "report: {report:?}");
        assert_eq!(report.covered_bytes, bytes.len() as u64);
    }

    #[test]
    fn unknown_field_type_is_recorded_under_coverage() {
        // One IFD entry with an unrecognised field-type code (13). The plain reader skips it and the
        // coverage reader records it; the 12-byte entry is part of the IFD body, so the file stays
        // fully covered.
        let data = [
            b'I', b'I', 0x2a, 0x00, 0x08, 0x00, 0x00, 0x00, // header, first IFD @ 8
            0x01, 0x00, // entry count = 1
            0x99, 0x99, // tag 0x9999
            0x0d, 0x00, // type 13 (unknown)
            0x01, 0x00, 0x00, 0x00, // value count = 1
            0x00, 0x00, 0x00, 0x00, // value/offset
            0x00, 0x00, 0x00, 0x00, // next IFD = 0
        ];
        let mut cov = Coverage::new(data.len() as u64);
        let mut unknown = Vec::new();
        let parsed = read_with_coverage(&data, &mut cov, &mut unknown).expect("read");
        assert_eq!(parsed.ifds[0].fields().len(), 0); // entry was skipped
        assert_eq!(unknown.len(), 1);
        assert_eq!(unknown[0].tag, 0x9999);
        assert_eq!(unknown[0].type_code, 13);
        assert_eq!(unknown[0].ifd_offset, 8);
        assert_eq!(unknown[0].entry_offset, 10);
        assert_eq!(unknown[0].count, 1);
        assert!(cov.finish().is_fully_covered());
    }

    #[test]
    fn read_ifd_at_with_coverage_accounts_a_subifd() {
        // Root + one sub-IFD, each with an even-length out-of-line value: accounting the root chain
        // and then the child via the pointer must leave no byte unclaimed.
        let mut child = Ifd::new();
        child.set(256, Value::Short(vec![16]));
        child.set(258, Value::Short(vec![8, 8, 8])); // 6 bytes -> out of line
        let mut root = Ifd::new();
        root.set(256, Value::Short(vec![640]));
        root.set(258, Value::Short(vec![8, 8, 8])); // 6 bytes -> out of line
        root.set_sub_ifd(330, vec![child]);
        let bytes = crate::write(&TiffFile {
            order: ByteOrder::LittleEndian,
            variant: Variant::Classic,
            ifds: vec![root],
        });

        let mut cov = Coverage::new(bytes.len() as u64);
        let mut unknown = Vec::new();
        let parsed = read_with_coverage(&bytes, &mut cov, &mut unknown).expect("root");
        let child_off = parsed.ifds[0].get_u32(330).expect("SubIFDs pointer");
        let (_child, next) = read_ifd_at_with_coverage(
            &bytes,
            child_off.into(),
            ByteOrder::LittleEndian,
            Variant::Classic,
            &mut cov,
            &mut unknown,
        )
        .expect("child");
        assert_eq!(next, 0);
        assert!(unknown.is_empty());
        let report = cov.finish();
        assert!(report.is_fully_covered(), "report: {report:?}");
    }
}
