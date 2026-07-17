//! Parsing of the TIFF byte-order header and the IFD chain.
//!
//! The structure is offset-driven — a classic parser-exploit surface — so every access is
//! bounds-checked, the IFD chain is guarded against loops and runaway length, and unknown field
//! types are preserved verbatim ([`Value::Unknown`]) rather than trusted or dropped.

use gamut_core::{Error, Result};

use crate::{ByteOrder, Coverage, Ifd, UnknownField, Value, Variant};

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

/// Guards a top-level next-IFD walk against loops and runaway length, shared by the slice
/// reader ([`read`]) and the streaming reader ([`IfdChain`](crate::IfdChain)) so the two paths
/// cannot drift.
///
/// A hash set keeps the loop guard O(1) per link: a hostile chain can be [`MAX_IFDS`] long, and
/// a linear scan per link would make it quadratic.
#[derive(Debug)]
pub(crate) struct ChainGuard {
    seen: std::collections::HashSet<u64>,
    count: usize,
}

impl ChainGuard {
    pub(crate) fn new() -> Self {
        Self {
            seen: std::collections::HashSet::new(),
            count: 0,
        }
    }

    /// Admits the next directory offset into the walk, or rejects a repeated offset (a loop) or
    /// the [`MAX_IFDS`] + 1'th directory (runaway length).
    pub(crate) fn admit(&mut self, offset: u64) -> Result<()> {
        if !self.seen.insert(offset) {
            return Err(Error::InvalidInput("TIFF: IFD chain loops"));
        }
        if self.count >= MAX_IFDS {
            return Err(Error::InvalidInput("TIFF: too many IFDs"));
        }
        self.count += 1;
        Ok(())
    }
}

/// Reads a 16-bit value at `pos` in `order`, bounds-checked.
pub(crate) fn u16_at(data: &[u8], pos: usize, order: ByteOrder) -> Result<u16> {
    let b = data
        .get(pos..pos + 2)
        .ok_or(Error::InvalidInput("TIFF: truncated 16-bit field"))?;
    Ok(order.u16([b[0], b[1]]))
}

/// Reads a 32-bit value at `pos` in `order`, bounds-checked.
pub(crate) fn u32_at(data: &[u8], pos: usize, order: ByteOrder) -> Result<u32> {
    let b = data
        .get(pos..pos + 4)
        .ok_or(Error::InvalidInput("TIFF: truncated 32-bit field"))?;
    Ok(order.u32([b[0], b[1], b[2], b[3]]))
}

/// Reads a 64-bit value at `pos` in `order`, bounds-checked (BigTIFF offsets/counts).
#[cfg(feature = "bigtiff")]
pub(crate) fn u64_at(data: &[u8], pos: usize, order: ByteOrder) -> Result<u64> {
    let b = data
        .get(pos..pos + 8)
        .ok_or(Error::InvalidInput("TIFF: truncated 64-bit field"))?;
    Ok(order.u64([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
}

/// Reads an offset-sized field at `pos` (a `u32` in classic TIFF, a `u64` in BigTIFF) as `u64`.
///
/// Used for every file offset and for the per-field value count, which share the offset width.
pub(crate) fn offset_at(
    data: &[u8],
    pos: usize,
    order: ByteOrder,
    variant: Variant,
) -> Result<u64> {
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
    let mut reader = crate::IfdReader::with_layout(data, order, variant);
    let raw = reader.read_ifd(offset)?;
    reader.decode_ifd(&raw)
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
    crate::IfdReader::with_layout(data, order, variant)
        .read_ifd_at_with_coverage(offset, cov, unknown)
}

/// Parses a TIFF/IFD stream: the header followed by the whole IFD chain.
///
/// This is a thin wrapper over the streaming engine ([`IfdReader`](crate::IfdReader)) — there is
/// exactly **one** parser, so the slice and streaming paths cannot drift.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if the header is invalid, an offset is out of bounds, the IFD
/// chain loops, or a field value is truncated.
pub fn read(data: &[u8]) -> Result<TiffFile> {
    crate::IfdReader::open(data)?.read_file()
}

/// An upper bound on the sub-IFD nesting depth [`read_tree`] follows, bounding hostile pointer
/// graphs. The deepest legitimate trees (a DNG raw sub-IFD, or EXIF's Exif → Interop chain) are
/// three levels.
const MAX_SUBIFD_DEPTH: usize = 16;

/// The file offsets a sub-IFD pointer value carries: a `LONG` array (TIFF 6.0 §2), the typed
/// `IFD` (13) form of TIFF Technical Note 1, or the 64-bit `LONG8`/`IFD8` forms BigTIFF writers
/// use. Any other type is not a pointer.
pub(crate) fn pointer_offsets(value: &Value) -> Option<Vec<u64>> {
    match value {
        Value::Long(v) | Value::Ifd(v) => Some(v.iter().map(|&x| u64::from(x)).collect()),
        #[cfg(feature = "bigtiff")]
        Value::Long8(v) | Value::Ifd8(v) => Some(v.clone()),
        _ => None,
    }
}

/// Like [`read`], but additionally follows the given sub-IFD **pointer tags**, reconstructing the
/// directory tree that [`write`](crate::write) flattens: each pointer field is replaced by a
/// [`sub_ifds`](Ifd::sub_ifds) group holding its parsed children, recursively.
///
/// The caller names the pointer tags because the structure alone cannot: which `LONG` tags hold
/// directory offsets is tag *semantics* (e.g. `SubIFDs` 330, `ExifIFD` 34665, `GPSInfo` 34853,
/// `Interoperability` 40965), which lives in the consuming codec. A named tag whose value is not
/// an offset array (`LONG`, or BigTIFF `LONG8`/`IFD8`) is left in place as a regular field.
///
/// This is [`write`](crate::write)'s inverse: `read_tree(&write(&file)?, tags)? == file` for any
/// tree whose pointer tags are all named in `tags`. For per-pointer control (e.g. tolerating a
/// malformed child), follow pointers manually with [`read_ifd_at`].
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] under the same conditions as [`read`], or if the pointer graph
/// is not a tree: a repeated or self-referential child offset, or nesting deeper than 16 levels.
pub fn read_tree(data: &[u8], pointer_tags: &[u16]) -> Result<TiffFile> {
    crate::IfdReader::open(data)?.read_tree(pointer_tags)
}

/// Follows each of `tags` in `ifd` (and, recursively, in the children), replacing the pointer
/// field with a parsed sub-IFD group. `visited` spans the whole walk so a hostile pointer graph
/// (a cycle, or two pointers claiming one directory) fails instead of looping.
///
/// `fetch` parses the directory at an offset — a closure over [`read_ifd_at`] for the slice
/// path, or over [`IfdReader::read_ifd`](crate::IfdReader::read_ifd) for the streaming path —
/// so the depth and cycle guards exist exactly once.
pub(crate) fn resolve_pointers_with(
    fetch: &mut dyn FnMut(u64) -> Result<Ifd>,
    ifd: &mut Ifd,
    tags: &[u16],
    visited: &mut Vec<u64>,
    depth: usize,
) -> Result<()> {
    if depth > MAX_SUBIFD_DEPTH {
        return Err(Error::InvalidInput("TIFF: sub-IFD tree too deep"));
    }
    for &tag in tags {
        let Some(offsets) = ifd.get(tag).and_then(pointer_offsets) else {
            continue;
        };
        let mut children = Vec::with_capacity(offsets.len());
        for off in offsets {
            if visited.contains(&off) {
                return Err(Error::InvalidInput("TIFF: sub-IFD pointer loop"));
            }
            visited.push(off);
            let mut child = fetch(off)?;
            resolve_pointers_with(fetch, &mut child, tags, visited, depth + 1)?;
            children.push(child);
        }
        ifd.remove(tag);
        ifd.set_sub_ifd(tag, children);
    }
    Ok(())
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
    crate::IfdReader::open(data)?.read_file_with_coverage(cov, unknown)
}

/// Like [`read`], but returns a dual-ledger-checked [`SegmentReport`](crate::SegmentReport)
/// alongside the parse: the whole file is read through a [`Tracked`](crate::Tracked) source,
/// the parser's typed claims are collected into a [`SegmentMap`](crate::SegmentMap), and the
/// two are cross-checked — so the report *proves* what the parse touched.
///
/// This audits the header and the **top-level chain** only; sub-IFDs a codec follows, its
/// strip/tile data extents, and padding classification are the codec-level audit's job (each
/// sub-IFD via [`IfdReader::read_ifd_at_audited`](crate::IfdReader::read_ifd_at_audited) over
/// the same map).
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] under the same conditions as [`read`].
pub fn read_audited(data: &[u8]) -> Result<(TiffFile, crate::SegmentReport)> {
    let mut tracked = crate::Tracked::new(data);
    let mut map = crate::SegmentMap::new(data.len() as u64);
    let file = crate::IfdReader::open(&mut tracked)?.read_file_audited(&mut map)?;
    Ok((file, map.finish(Some(tracked.ledger()))))
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
        let bytes = crate::write(&file).expect("write");
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
    fn unknown_field_type_is_preserved_and_recorded_under_coverage() {
        // One IFD entry with an unrecognised field-type code (0xF0). The readers preserve the
        // entry verbatim as a `Value::Unknown` (nothing is dropped on a rewrite), the coverage
        // reader additionally records it, and the 12-byte entry is part of the IFD body, so the
        // file stays fully covered.
        let data = [
            b'I', b'I', 0x2a, 0x00, 0x08, 0x00, 0x00, 0x00, // header, first IFD @ 8
            0x01, 0x00, // entry count = 1
            0x99, 0x99, // tag 0x9999
            0xf0, 0x00, // type 0xF0 (unknown)
            0x01, 0x00, 0x00, 0x00, // value count = 1
            0xde, 0xad, 0xbe, 0xef, // value/offset word (opaque)
            0x00, 0x00, 0x00, 0x00, // next IFD = 0
        ];
        let mut cov = Coverage::new(data.len() as u64);
        let mut unknown = Vec::new();
        let parsed = read_with_coverage(&data, &mut cov, &mut unknown).expect("read");
        // The entry is preserved, not skipped: the raw record survives in the Ifd.
        assert_eq!(parsed.ifds[0].fields().len(), 1);
        let Some(Value::Unknown(u)) = parsed.ifds[0].get(0x9999) else {
            panic!("expected a preserved Value::Unknown");
        };
        assert_eq!(u.type_code(), 0xF0);
        assert_eq!(u.count(), 1);
        assert_eq!(u.word(), &[0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(u.order(), ByteOrder::LittleEndian);
        assert_eq!(u.variant(), Variant::Classic);
        // The plain reader agrees (preservation is not a coverage-path special case).
        assert_eq!(read(&data).expect("plain read"), parsed);
        assert_eq!(unknown.len(), 1);
        assert_eq!(unknown[0].tag, 0x9999);
        assert_eq!(unknown[0].type_code, 0xF0);
        assert_eq!(unknown[0].ifd_offset, 8);
        assert_eq!(unknown[0].entry_offset, 10);
        assert_eq!(unknown[0].count, 1);
        assert!(cov.finish().is_fully_covered());
    }

    /// The keystone symmetry: `read_tree` is `write`'s inverse over a nested sub-IFD tree — the
    /// pointer fields disappear, the children come back in place, and the whole `TiffFile`
    /// compares equal.
    fn read_tree_inverts_write(order: ByteOrder, variant: Variant) {
        let mut grandchild = Ifd::new();
        grandchild.set(33434, Value::Rational(vec![(1, 200)])); // ExposureTime
        let mut raw_a = Ifd::new();
        raw_a.set(256, Value::Short(vec![16]));
        raw_a.set_sub_ifd(34665, vec![grandchild]);
        let mut raw_b = Ifd::new();
        raw_b.set(256, Value::Short(vec![8]));
        let mut root = Ifd::new();
        root.set(256, Value::Short(vec![640]));
        root.set(258, Value::Short(vec![8, 8, 8])); // out of line
        root.set_sub_ifd(330, vec![raw_a, raw_b]);
        let file = TiffFile {
            order,
            variant,
            ifds: vec![root],
        };
        let bytes = crate::write(&file).expect("write");
        let tree = read_tree(&bytes, &[330, 34665]).expect("read_tree");
        assert_eq!(tree, file);
        // The pointer tags were consumed into sub_ifds, not left as stale offset fields.
        assert_eq!(tree.ifds[0].get(330), None);
    }

    #[test]
    fn read_tree_inverts_write_classic_both_orders() {
        read_tree_inverts_write(ByteOrder::LittleEndian, Variant::Classic);
        read_tree_inverts_write(ByteOrder::BigEndian, Variant::Classic);
    }

    #[cfg(feature = "bigtiff")]
    #[test]
    fn read_tree_inverts_write_bigtiff() {
        read_tree_inverts_write(ByteOrder::LittleEndian, Variant::Big);
    }

    /// A named pointer tag whose value is not an offset array is left alone as a regular field
    /// (the type check TIFF 6.0 §2 asks of readers), and unnamed `LONG` tags are never followed.
    #[test]
    fn read_tree_leaves_non_pointer_values_in_place() {
        let mut root = Ifd::new();
        root.set(330, Value::Ascii("not a pointer".to_owned()));
        root.set(700, Value::Long(vec![8])); // a plausible offset, but not a named pointer tag
        let file = TiffFile {
            order: ByteOrder::LittleEndian,
            variant: Variant::Classic,
            ifds: vec![root.clone()],
        };
        let bytes = crate::write(&file).expect("write");
        let tree = read_tree(&bytes, &[330]).expect("read_tree");
        assert_eq!(tree.ifds[0], root);
    }

    /// A pointer stored with the typed `IFD` (13) field type — TIFF Technical Note 1's form —
    /// is followed by `read_tree` exactly like a `LONG` pointer. Hand-built, since `write`
    /// synthesises `LONG` pointers.
    #[test]
    fn typed_ifd_pointer_is_followed_by_read_tree() {
        let data = [
            b'I', b'I', 0x2a, 0x00, 0x08, 0x00, 0x00, 0x00, // header, first IFD @ 8
            0x01, 0x00, // IFD0: entry count = 1
            0x4a, 0x01, // tag 330 (SubIFDs)
            0x0d, 0x00, // type 13 (IFD)
            0x01, 0x00, 0x00, 0x00, // count = 1
            0x1a, 0x00, 0x00, 0x00, // offset 26 (the child directory)
            0x00, 0x00, 0x00, 0x00, // next IFD = 0
            0x01, 0x00, // child @ 26: entry count = 1
            0x00, 0x01, // tag 256
            0x03, 0x00, // type 3 (SHORT)
            0x01, 0x00, 0x00, 0x00, // count = 1
            0x07, 0x00, 0x00, 0x00, // value 7 (inline)
            0x00, 0x00, 0x00, 0x00, // next IFD = 0
        ];
        // The flat reader decodes the pointer as a typed value, not a followed directory.
        let flat = read(&data).expect("read");
        assert_eq!(flat.ifds[0].get(330), Some(&Value::Ifd(vec![26])));
        // read_tree follows it like a LONG pointer.
        let tree = read_tree(&data, &[330]).expect("read_tree");
        assert_eq!(
            tree.ifds[0].get(330),
            None,
            "pointer consumed into the tree"
        );
        let children = &tree.ifds[0].sub_ifds()[0];
        assert_eq!(children.tag, 330);
        assert_eq!(children.ifds[0].get_u32(256), Some(7));
    }

    /// Adversarial: a sub-IFD pointer aimed at its own directory must terminate with a typed
    /// error (the visited set), not recurse forever.
    #[test]
    fn read_tree_rejects_self_pointing_sub_ifd() {
        // Root IFD at 8 with one LONG field, tag 330, whose value is a child offset — pointing
        // at offset 26, which is a directory whose own tag-330 pointer points back at 26.
        let data = [
            b'I', b'I', 0x2a, 0x00, 0x08, 0x00, 0x00, 0x00, // header, first IFD @ 8
            0x01, 0x00, // IFD0: entry count = 1
            0x4a, 0x01, // tag 330
            0x04, 0x00, // type 4 (LONG)
            0x01, 0x00, 0x00, 0x00, // count = 1
            0x1a, 0x00, 0x00, 0x00, // offset 26 (the child directory)
            0x00, 0x00, 0x00, 0x00, // next IFD = 0
            0x01, 0x00, // child @ 26: entry count = 1
            0x4a, 0x01, // tag 330
            0x04, 0x00, // type 4 (LONG)
            0x01, 0x00, 0x00, 0x00, // count = 1
            0x1a, 0x00, 0x00, 0x00, // offset 26 — itself
            0x00, 0x00, 0x00, 0x00, // next IFD = 0
        ];
        assert!(read(&data).is_ok()); // the flat reader is untouched by the cycle
        assert!(read_tree(&data, &[330]).is_err());
    }

    /// A tree at exactly the depth cap parses — pinning the guard's boundary against the
    /// depth-bomb rejection below.
    #[test]
    fn read_tree_allows_exactly_max_depth() {
        let mut ifd = Ifd::new();
        ifd.set(256, Value::Short(vec![1]));
        // 15 nestings put the deepest directory at depth 16 (the root is depth 1).
        for _ in 0..15 {
            let mut parent = Ifd::new();
            parent.set_sub_ifd(330, vec![ifd]);
            ifd = parent;
        }
        let bytes = crate::write(&TiffFile {
            order: ByteOrder::LittleEndian,
            variant: Variant::Classic,
            ifds: vec![ifd],
        })
        .expect("write");
        assert!(read_tree(&bytes, &[330]).is_ok());
    }

    /// Adversarial: nesting past the depth cap is a typed error, not a stack overflow.
    #[test]
    fn read_tree_rejects_depth_bomb() {
        let mut ifd = Ifd::new();
        ifd.set(256, Value::Short(vec![1]));
        for _ in 0..17 {
            let mut parent = Ifd::new();
            parent.set_sub_ifd(330, vec![ifd]);
            ifd = parent;
        }
        let bytes = crate::write(&TiffFile {
            order: ByteOrder::LittleEndian,
            variant: Variant::Classic,
            ifds: vec![ifd],
        })
        .expect("write");
        assert!(read_tree(&bytes, &[330]).is_err());
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
        })
        .expect("write");

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
