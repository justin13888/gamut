//! Serialisation of the TIFF byte-order header and the IFD tree.
//!
//! The writer lays the stream out in two passes: it first computes where each IFD and each
//! out-of-line value lands (header → IFDs → value pool, every block on an even/word boundary),
//! then emits the bytes with the absolute offsets patched in. Values that fit in the entry's
//! value/offset field — four bytes in classic TIFF, eight in BigTIFF — are packed inline,
//! left-justified (TIFF 6.0 §2); the [`Variant`] selects every structural field width.
//!
//! This two-pass offset layout is the crate's **keystone**: out-of-line values and following IFDs
//! need absolute offsets that are only known once sizes are fixed, so the layout is planned then
//! the offset words are back-patched. A read → write → read round-trip reproduces the directory
//! exactly.
//!
//! ## Sub-IFD trees
//!
//! Beyond the top-level next-IFD chain ([`TiffFile::ifds`]), an [`Ifd`] may carry
//! [`sub_ifds`](Ifd::sub_ifds): child directories referenced by a pointer *tag* rather than the
//! chain (e.g. a DNG's raw sub-IFD via `SubIFDs`, or an `ExifIFD`). The layout generalises to the
//! whole **tree** — every directory (top-level and descendant) is placed first, then a single
//! value pool — and each pointer tag's value is synthesised as a `LONG`/`LONG8` array of its
//! children's offsets. With no sub-IFDs this reduces to the flat chain, byte-for-byte.

use crate::{ByteOrder, Ifd, TiffFile, Value, Variant};

/// Rounds `n` up to the next even (word) boundary, as required for value offsets.
fn even(n: usize) -> usize {
    n + (n & 1)
}

/// A field value to emit: either borrowed from the directory, or a sub-IFD pointer-offset array the
/// writer synthesises once the child directories have been placed.
enum FieldRef<'a> {
    /// A real field value, borrowed from the source [`Ifd`].
    Real(&'a Value),
    /// A synthesised `SubIFDs`/`ExifIFD`-style pointer: the offsets of the child directories.
    Synth(Value),
}

impl FieldRef<'_> {
    fn value(&self) -> &Value {
        match self {
            FieldRef::Real(v) => v,
            FieldRef::Synth(v) => v,
        }
    }
}

/// One IFD flattened out of the tree, with its placement bookkeeping.
struct Node<'a> {
    /// The source directory (for its real fields).
    ifd: &'a Ifd,
    /// Each sub-IFD pointer as `(tag, child node indices)`, in `sub_ifds` order.
    pointers: Vec<(u16, Vec<usize>)>,
    /// The next directory in the top-level chain (`None` for descendants and the last page).
    next: Option<usize>,
    /// Number of on-disk entries: real fields plus one synthesised entry per pointer.
    n_entries: usize,
    /// The directory's absolute file offset (assigned in pass 1).
    offset: u64,
}

/// Appends `ifd` and its descendants to `nodes` (parent before children), returning `ifd`'s index.
fn push_node<'a>(ifd: &'a Ifd, nodes: &mut Vec<Node<'a>>) -> usize {
    let idx = nodes.len();
    nodes.push(Node {
        ifd,
        pointers: Vec::new(),
        next: None,
        n_entries: 0,
        offset: 0,
    });
    let mut pointers = Vec::with_capacity(ifd.sub_ifds().len());
    for sub in ifd.sub_ifds() {
        let children: Vec<usize> = sub.ifds.iter().map(|c| push_node(c, nodes)).collect();
        pointers.push((sub.tag, children));
    }
    nodes[idx].n_entries = ifd.fields().len() + pointers.len();
    nodes[idx].pointers = pointers;
    idx
}

/// Builds a pointer field value (a child-offset array) of the right type for `variant`.
fn pointer_value(variant: Variant, offsets: Vec<u64>) -> Value {
    match variant {
        Variant::Classic => Value::Long(offsets.iter().map(|&o| o as u32).collect()),
        #[cfg(feature = "bigtiff")]
        Variant::Big => Value::Long8(offsets),
    }
}

/// Writes an offset-sized integer (`u32` classic / `u64` BigTIFF) at `pos`, used for every file
/// offset and the per-field value count, which share the offset width.
fn put_offset(out: &mut [u8], pos: usize, v: u64, order: ByteOrder, variant: Variant) {
    match variant {
        Variant::Classic => out[pos..pos + 4].copy_from_slice(&order.pack_u32(v as u32)),
        #[cfg(feature = "bigtiff")]
        Variant::Big => out[pos..pos + 8].copy_from_slice(&order.pack_u64(v)),
    }
}

/// Serialises a TIFF/IFD stream (header + IFD tree + out-of-line value pool) to bytes.
///
/// Top-level IFDs ([`TiffFile::ifds`]) are written in order and linked through their next-IFD
/// pointers; any [`sub_ifds`](Ifd::sub_ifds) are laid out as additional directories and referenced
/// by a synthesised pointer field. Out-of-line values are appended in a value pool after the
/// directories. Image/pixel data is not handled here — a codec composes that around this primitive
/// (see `gamut-tiff`'s strip/tile writers).
#[must_use]
pub fn write(file: &TiffFile) -> Vec<u8> {
    let order = file.order;
    let variant = file.variant;
    let entry_size = variant.entry_size();
    let offset_size = variant.offset_size();
    let inline = variant.inline_threshold();

    // Flatten the tree, then link the top-level directories through the next-IFD chain.
    let mut nodes: Vec<Node> = Vec::new();
    let top: Vec<usize> = file
        .ifds
        .iter()
        .map(|ifd| push_node(ifd, &mut nodes))
        .collect();
    for pair in top.windows(2) {
        nodes[pair[0]].next = Some(pair[1]);
    }

    // Pass 1a: place every directory block (top-level and descendant), each on a word boundary.
    let mut cursor = even(variant.header_size());
    for node in &mut nodes {
        node.offset = cursor as u64;
        cursor = even(cursor + variant.count_size() + node.n_entries * entry_size + offset_size);
    }

    // With every directory offset known, synthesise each pointer field (a child-offset array) and
    // build each directory's tag-sorted entry list (real fields interleaved with pointers).
    let mut entries_per_node: Vec<Vec<(u16, FieldRef)>> = Vec::with_capacity(nodes.len());
    for node in &nodes {
        let mut entries: Vec<(u16, FieldRef)> = Vec::with_capacity(node.n_entries);
        for field in node.ifd.fields() {
            entries.push((field.tag, FieldRef::Real(&field.value)));
        }
        for (tag, children) in &node.pointers {
            let offsets = children.iter().map(|&ci| nodes[ci].offset).collect();
            entries.push((*tag, FieldRef::Synth(pointer_value(variant, offsets))));
        }
        entries.sort_by_key(|(tag, _)| *tag);
        entries_per_node.push(entries);
    }

    // Pass 1b: place the out-of-line value pool after the directories.
    let mut value_offsets: Vec<Vec<u64>> = Vec::with_capacity(nodes.len());
    let mut pool: Vec<(usize, Vec<u8>)> = Vec::new();
    for entries in &entries_per_node {
        let mut offs = Vec::with_capacity(entries.len());
        for (_tag, field) in entries {
            let byte_len = field.value().byte_len();
            if byte_len <= inline {
                offs.push(0);
            } else {
                cursor = even(cursor);
                offs.push(cursor as u64);
                pool.push((cursor, field.value().encode(order)));
                cursor += byte_len;
            }
        }
        value_offsets.push(offs);
    }

    // Pass 2: emit.
    let mut out = vec![0u8; cursor];
    out[0..2].copy_from_slice(match order {
        ByteOrder::LittleEndian => b"II",
        ByteOrder::BigEndian => b"MM",
    });
    out[2..4].copy_from_slice(&order.pack_u16(variant.magic()));
    let first = nodes.first().map_or(0, |n| n.offset);
    match variant {
        Variant::Classic => out[4..8].copy_from_slice(&order.pack_u32(first as u32)),
        #[cfg(feature = "bigtiff")]
        Variant::Big => {
            // Bytes 4-5 are the offset bytesize (8), 6-7 are reserved (0, already zeroed), and
            // the first-IFD offset is the 8-byte value at bytes 8-15.
            out[4..6].copy_from_slice(&order.pack_u16(8));
            out[8..16].copy_from_slice(&order.pack_u64(first));
        }
    }

    for (idx, entries) in entries_per_node.iter().enumerate() {
        let node = &nodes[idx];
        let mut pos = node.offset as usize;
        let n = entries.len();
        match variant {
            Variant::Classic => out[pos..pos + 2].copy_from_slice(&order.pack_u16(n as u16)),
            #[cfg(feature = "bigtiff")]
            Variant::Big => out[pos..pos + 8].copy_from_slice(&order.pack_u64(n as u64)),
        }
        pos += variant.count_size();
        for ((tag, field), &voff) in entries.iter().zip(&value_offsets[idx]) {
            let value = field.value();
            let bytes = value.encode(order);
            out[pos..pos + 2].copy_from_slice(&order.pack_u16(*tag));
            out[pos + 2..pos + 4].copy_from_slice(&order.pack_u16(value.field_type().code()));
            put_offset(&mut out, pos + 4, value.count() as u64, order, variant);
            let value_pos = pos + 4 + offset_size;
            if bytes.len() <= inline {
                // Inline, left-justified: low bytes hold the value, remainder is zero.
                out[value_pos..value_pos + bytes.len()].copy_from_slice(&bytes);
            } else {
                put_offset(&mut out, value_pos, voff, order, variant);
            }
            pos += entry_size;
        }
        let next = node.next.map_or(0, |ni| nodes[ni].offset);
        put_offset(&mut out, pos, next, order, variant);
    }

    for (offset, bytes) in pool {
        out[offset..offset + bytes.len()].copy_from_slice(&bytes);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{read, read_header, read_ifd_at};

    // Tag numbers are used literally: tag semantics live in the consuming codec, not this
    // structural core. 256/257 = ImageWidth/ImageLength, 258 = BitsPerSample, 282 = XResolution.
    fn sample_ifd() -> Ifd {
        let mut ifd = Ifd::new();
        ifd.set(256, Value::Short(vec![640]));
        ifd.set(257, Value::Long(vec![480]));
        ifd.set(258, Value::Short(vec![8, 8, 8])); // 6 bytes -> out of line (classic)
        ifd.set(282, Value::Rational(vec![(300, 1)])); // 8 bytes -> out of line (classic)
        ifd.set(72, Value::Ascii("gamut-tiff".to_owned())); // out of line
        ifd
    }

    fn roundtrip(order: ByteOrder, variant: Variant) {
        let file = TiffFile {
            order,
            variant,
            ifds: vec![sample_ifd()],
        };
        let bytes = write(&file);
        let parsed = read(&bytes).expect("read back");
        assert_eq!(parsed, file);
    }

    fn multi_ifd_roundtrip(variant: Variant) {
        let mut second = Ifd::new();
        second.set(256, Value::Short(vec![1]));
        second.set(257, Value::Short(vec![1]));
        let file = TiffFile {
            order: ByteOrder::LittleEndian,
            variant,
            ifds: vec![sample_ifd(), second],
        };
        let bytes = write(&file);
        let parsed = read(&bytes).expect("read back");
        assert_eq!(parsed.ifds.len(), 2);
        assert_eq!(parsed, file);
    }

    #[test]
    fn classic_single_ifd_roundtrips_both_orders() {
        roundtrip(ByteOrder::LittleEndian, Variant::Classic);
        roundtrip(ByteOrder::BigEndian, Variant::Classic);
    }

    #[test]
    fn classic_multi_ifd_chain_roundtrips() {
        multi_ifd_roundtrip(Variant::Classic);
    }

    #[test]
    fn write_layout_is_tight() {
        // The exact stream length pins the two-pass cursor math — `ifd_size`, the per-IFD advance,
        // and the value-pool append — that a read->write->read round-trip can't see: the reader
        // follows stored offsets, so a too-large IFD size or a wrong cursor step only inserts gaps
        // it still parses back correctly.
        let one = write(&TiffFile {
            order: ByteOrder::LittleEndian,
            variant: Variant::Classic,
            ifds: vec![sample_ifd()],
        });
        assert_eq!(one.len(), 100);
        let mut second = Ifd::new();
        second.set(256, Value::Short(vec![1]));
        second.set(257, Value::Short(vec![1]));
        let two = write(&TiffFile {
            order: ByteOrder::LittleEndian,
            variant: Variant::Classic,
            ifds: vec![sample_ifd(), second],
        });
        assert_eq!(two.len(), 130);
    }

    #[test]
    fn value_offsets_are_even() {
        // BitsPerSample (6 bytes) forces an out-of-line value; its offset must be word-aligned.
        let bytes = write(&TiffFile {
            order: ByteOrder::LittleEndian,
            variant: Variant::Classic,
            ifds: vec![sample_ifd()],
        });
        // Header magic and first-IFD offset are well-formed.
        assert_eq!(&bytes[0..2], b"II");
        assert_eq!(read_header(&bytes).expect("header").2, 8);
    }

    /// A directory with no sub-IFDs must serialise byte-for-byte as it did before tree support, so
    /// the flat path (and `gamut-tiff`'s libtiff oracle) is unaffected.
    #[test]
    fn flat_layout_is_unchanged_by_tree_support() {
        let file = TiffFile {
            order: ByteOrder::LittleEndian,
            variant: Variant::Classic,
            ifds: vec![sample_ifd()],
        };
        let bytes = write(&file);
        // Golden layout: 8-byte header, IFD0 at 8 (5 entries), value pool after. Re-reading must
        // reproduce the directory exactly, and there is no second (sub-)IFD in the chain.
        let (_order, _variant, first) = read_header(&bytes).expect("header");
        assert_eq!(first, 8);
        assert_eq!(read(&bytes).expect("read").ifds.len(), 1);
    }

    fn subifd_tree_roundtrips(order: ByteOrder, variant: Variant) {
        // IFD0 carries a raw sub-IFD (tag 330) with two children and an EXIF sub-IFD (tag 34665).
        let mut raw_a = Ifd::new();
        raw_a.set(256, Value::Short(vec![16]));
        raw_a.set(257, Value::Short(vec![16]));
        raw_a.set(254, Value::Long(vec![0])); // NewSubFileType = full-resolution
        let mut raw_b = Ifd::new();
        raw_b.set(256, Value::Short(vec![8]));
        raw_b.set(257, Value::Short(vec![8]));
        let mut exif = Ifd::new();
        exif.set(33434, Value::Rational(vec![(1, 100)])); // ExposureTime

        let mut root = sample_ifd();
        root.set_sub_ifd(330, vec![raw_a.clone(), raw_b.clone()]);
        root.set_sub_ifd(34665, vec![exif.clone()]);

        let file = TiffFile {
            order,
            variant,
            ifds: vec![root],
        };
        let bytes = write(&file);

        // The generic reader returns just the top-level chain (the children are not chained).
        let parsed = read(&bytes).expect("read back");
        assert_eq!(parsed.ifds.len(), 1);
        let root_ifd = &parsed.ifds[0];
        // Its real fields survive...
        assert_eq!(root_ifd.get(256), Some(&Value::Short(vec![640])));
        // ...and the synthesised pointer tags are present as offset arrays.
        let sub_offsets = root_ifd.get_u32_vec(330).expect("SubIFDs pointer");
        assert_eq!(sub_offsets.len(), 2);
        let exif_offset = root_ifd.get_u32(34665).expect("ExifIFD pointer");

        // Following the pointers re-parses the children exactly.
        assert_eq!(
            read_ifd_at(&bytes, sub_offsets[0].into(), order, variant).unwrap(),
            raw_a
        );
        assert_eq!(
            read_ifd_at(&bytes, sub_offsets[1].into(), order, variant).unwrap(),
            raw_b
        );
        assert_eq!(
            read_ifd_at(&bytes, exif_offset.into(), order, variant).unwrap(),
            exif
        );
    }

    #[test]
    fn classic_subifd_tree_roundtrips_both_orders() {
        subifd_tree_roundtrips(ByteOrder::LittleEndian, Variant::Classic);
        subifd_tree_roundtrips(ByteOrder::BigEndian, Variant::Classic);
    }

    #[test]
    fn nested_subifd_tree_roundtrips() {
        // A grandchild: IFD0 -> SubIFD -> its own (e.g. EXIF) sub-IFD, exercising recursion.
        let mut grandchild = Ifd::new();
        grandchild.set(33434, Value::Rational(vec![(1, 200)]));
        let mut child = Ifd::new();
        child.set(256, Value::Short(vec![32]));
        child.set_sub_ifd(34665, vec![grandchild.clone()]);
        let mut root = Ifd::new();
        root.set(256, Value::Short(vec![64]));
        root.set_sub_ifd(330, vec![child]);

        let file = TiffFile {
            order: ByteOrder::LittleEndian,
            variant: Variant::Classic,
            ifds: vec![root],
        };
        let bytes = write(&file);
        let parsed = read(&bytes).expect("read");
        let child_off = parsed.ifds[0].get_u32(330).expect("SubIFDs");
        let child_ifd = read_ifd_at(
            &bytes,
            child_off.into(),
            ByteOrder::LittleEndian,
            Variant::Classic,
        )
        .expect("child");
        assert_eq!(child_ifd.get(256), Some(&Value::Short(vec![32])));
        let gc_off = child_ifd.get_u32(34665).expect("nested ExifIFD");
        let gc = read_ifd_at(
            &bytes,
            gc_off.into(),
            ByteOrder::LittleEndian,
            Variant::Classic,
        )
        .expect("grandchild");
        assert_eq!(gc, grandchild);
    }

    #[cfg(feature = "bigtiff")]
    #[test]
    fn bigtiff_roundtrips_and_inline_threshold() {
        roundtrip(ByteOrder::LittleEndian, Variant::Big);
        roundtrip(ByteOrder::BigEndian, Variant::Big);
        multi_ifd_roundtrip(Variant::Big);
        subifd_tree_roundtrips(ByteOrder::LittleEndian, Variant::Big);

        // The 8-byte XResolution rational is out of line in classic TIFF (>4 B) but packs inline
        // in BigTIFF (<=8 B); both must round-trip identically.
        let bytes = write(&TiffFile {
            order: ByteOrder::LittleEndian,
            variant: Variant::Big,
            ifds: vec![sample_ifd()],
        });
        assert_eq!(&bytes[0..2], b"II");
        let (order, variant, first) = read_header(&bytes).expect("header");
        assert_eq!(order, ByteOrder::LittleEndian);
        assert_eq!(variant, Variant::Big);
        assert_eq!(bytes[2], 0x2b); // magic 43
        assert_eq!(first, 16); // 16-byte header
        let parsed = read(&bytes).expect("read back");
        assert_eq!(parsed.variant, Variant::Big);
        assert_eq!(
            parsed.ifds[0].get(282),
            Some(&Value::Rational(vec![(300, 1)]))
        );
    }

    #[test]
    fn subifd_children_are_placed_consecutively_after_the_root() {
        // Pass-1a places each directory right after the previous one, word-aligned. With a single
        // inline field each, the sizes are exact, so the children land at fixed offsets — pinning the
        // per-directory cursor arithmetic (which round-trips can't, since wrong-but-consistent
        // offsets still parse).
        let mut child_a = Ifd::new();
        child_a.set(256, Value::Short(vec![1]));
        let mut child_b = Ifd::new();
        child_b.set(256, Value::Short(vec![1]));
        let mut root = Ifd::new();
        root.set(256, Value::Short(vec![1]));
        root.set_sub_ifd(330, vec![child_a, child_b]);

        let bytes = write(&TiffFile {
            order: ByteOrder::LittleEndian,
            variant: Variant::Classic,
            ifds: vec![root],
        });
        let offs = read(&bytes).expect("read").ifds[0]
            .get_u32_vec(330)
            .expect("SubIFDs pointer");
        // header(8) + root dir(count 2 + 2 entries*12 + next 4 = 30) -> child A at 38;
        // child A dir(2 + 1*12 + 4 = 18) -> child B at 56.
        assert_eq!(offs, vec![38, 56]);
    }

    #[test]
    fn out_of_line_value_pool_is_tightly_packed() {
        // sample_ifd carries several out-of-line values; each advances the cursor by its own length.
        // A mutated advance (e.g. `*=` for `+=`) would balloon the file far past this bound.
        let bytes = write(&TiffFile {
            order: ByteOrder::LittleEndian,
            variant: Variant::Classic,
            ifds: vec![sample_ifd()],
        });
        assert!(
            bytes.len() < 256,
            "value pool not tightly packed: {} bytes",
            bytes.len()
        );
    }
}
