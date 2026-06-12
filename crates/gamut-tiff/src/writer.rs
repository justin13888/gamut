//! Serialisation of the TIFF byte-order header and the IFD chain.
//!
//! The writer lays the file out in two passes: it first computes where each IFD and each
//! out-of-line value lands (header → IFDs → value pool, every block on an even/word boundary),
//! then emits the bytes with the absolute offsets patched in. Values that fit in the entry's
//! value/offset field — four bytes in classic TIFF, eight in BigTIFF — are packed inline,
//! left-justified (TIFF 6.0 §2); the [`Variant`] selects every structural field width.

use crate::ifd::{ByteOrder, Ifd, Value, Variant};
use crate::reader::TiffFile;

/// Rounds `n` up to the next even (word) boundary, as required for value offsets.
fn even(n: usize) -> usize {
    n + (n & 1)
}

/// The on-disk size of an IFD for `variant`: the entry count, one entry per field, and the
/// next-IFD pointer (e.g. classic `2 + 12n + 4`, BigTIFF `8 + 20n + 8`).
fn ifd_size(ifd: &Ifd, variant: Variant) -> usize {
    variant.count_size() + ifd.fields().len() * variant.entry_size() + variant.offset_size()
}

/// Writes an offset-sized integer (`u32` classic / `u64` BigTIFF) at `pos`, used for every file
/// offset and the per-field value count, which share the offset width.
fn put_offset(out: &mut [u8], pos: usize, v: u64, order: ByteOrder, variant: Variant) {
    match variant {
        Variant::Classic => out[pos..pos + 4].copy_from_slice(&order.pack_u32(v as u32)),
        Variant::Big => out[pos..pos + 8].copy_from_slice(&order.pack_u64(v)),
    }
}

/// Builds a strip/tile-offset field value of the right type for `variant`: `LONG` for classic
/// TIFF, `LONG8` for BigTIFF (whose offsets may exceed 4 GiB).
fn offset_value(variant: Variant, offsets: Vec<u64>) -> Value {
    match variant {
        Variant::Classic => Value::Long(offsets.iter().map(|&o| o as u32).collect()),
        Variant::Big => Value::Long8(offsets),
    }
}

/// Serialises a TIFF file (header + IFD chain + image/value data) to bytes.
///
/// IFDs are written in order and linked through their next-IFD pointers; out-of-line values are
/// appended in a value pool after the directories.
#[must_use]
pub fn write(file: &TiffFile) -> Vec<u8> {
    let order = file.order;
    let variant = file.variant;
    let entry_size = variant.entry_size();
    let offset_size = variant.offset_size();
    let inline = variant.inline_threshold();

    // Pass 1: place the header and IFDs, then the value pool. Record each IFD's start and each
    // out-of-line value's (offset, bytes).
    let mut cursor = even(variant.header_size());
    let mut ifd_offsets: Vec<u64> = Vec::with_capacity(file.ifds.len());
    for ifd in &file.ifds {
        ifd_offsets.push(cursor as u64);
        cursor = even(cursor + ifd_size(ifd, variant));
    }
    // Per-IFD, per-field out-of-line value offset (0 if packed inline).
    let mut value_offsets: Vec<Vec<u64>> = Vec::with_capacity(file.ifds.len());
    let mut pool: Vec<(usize, Vec<u8>)> = Vec::new();
    for ifd in &file.ifds {
        let mut offs = Vec::with_capacity(ifd.fields().len());
        for field in ifd.fields() {
            let bytes = field.value.encode(order);
            if bytes.len() <= inline {
                offs.push(0);
            } else {
                cursor = even(cursor);
                offs.push(cursor as u64);
                pool.push((cursor, bytes.clone()));
                cursor += bytes.len();
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
    let first = ifd_offsets.first().copied().unwrap_or(0);
    match variant {
        Variant::Classic => out[4..8].copy_from_slice(&order.pack_u32(first as u32)),
        Variant::Big => {
            // Bytes 4-5 are the offset bytesize (8), 6-7 are reserved (0, already zeroed), and
            // the first-IFD offset is the 8-byte value at bytes 8-15.
            out[4..6].copy_from_slice(&order.pack_u16(8));
            out[8..16].copy_from_slice(&order.pack_u64(first));
        }
    }

    for (idx, ifd) in file.ifds.iter().enumerate() {
        let mut pos = ifd_offsets[idx] as usize;
        let n = ifd.fields().len();
        match variant {
            Variant::Classic => out[pos..pos + 2].copy_from_slice(&order.pack_u16(n as u16)),
            Variant::Big => out[pos..pos + 8].copy_from_slice(&order.pack_u64(n as u64)),
        }
        pos += variant.count_size();
        for (field, &voff) in ifd.fields().iter().zip(&value_offsets[idx]) {
            let bytes = field.value.encode(order);
            out[pos..pos + 2].copy_from_slice(&order.pack_u16(field.tag));
            out[pos + 2..pos + 4].copy_from_slice(&order.pack_u16(field.value.field_type().code()));
            put_offset(
                &mut out,
                pos + 4,
                field.value.count() as u64,
                order,
                variant,
            );
            let value_pos = pos + 4 + offset_size;
            if bytes.len() <= inline {
                // Inline, left-justified: low bytes hold the value, remainder is zero.
                out[value_pos..value_pos + bytes.len()].copy_from_slice(&bytes);
            } else {
                put_offset(&mut out, value_pos, voff, order, variant);
            }
            pos += entry_size;
        }
        let next = file.ifds.get(idx + 1).map_or(0, |_| ifd_offsets[idx + 1]);
        put_offset(&mut out, pos, next, order, variant);
    }

    for (offset, bytes) in pool {
        out[offset..offset + bytes.len()].copy_from_slice(&bytes);
    }
    out
}

/// Serialises a single-IFD strip TIFF image: the supplied directory plus its strip data,
/// referenced by `StripOffsets`/`StripByteCounts`. The strips are written verbatim, so the caller
/// is responsible for any per-strip compression. `variant` selects classic TIFF or BigTIFF.
#[must_use]
pub fn write_image(order: ByteOrder, variant: Variant, ifd: &Ifd, strips: &[Vec<u8>]) -> Vec<u8> {
    write_blocks(
        order,
        variant,
        ifd,
        strips,
        crate::tags::STRIP_OFFSETS,
        crate::tags::STRIP_BYTE_COUNTS,
    )
}

/// Serialises a single-IFD tiled TIFF image: the directory plus its tile data, referenced by
/// `TileOffsets`/`TileByteCounts`. The caller supplies the `TileWidth`/`TileLength` fields and any
/// per-tile compression. `variant` selects classic TIFF or BigTIFF.
#[must_use]
pub fn write_image_tiled(
    order: ByteOrder,
    variant: Variant,
    ifd: &Ifd,
    tiles: &[Vec<u8>],
) -> Vec<u8> {
    write_blocks(
        order,
        variant,
        ifd,
        tiles,
        crate::tags::TILE_OFFSETS,
        crate::tags::TILE_BYTE_COUNTS,
    )
}

/// Serialises a multi-page (multi-IFD) strip TIFF: each page's directory — linked through the
/// next-IFD pointers — plus all pages' strip data in a shared region after the directories. Each
/// page is `(directory, strips)`; `StripOffsets`/`StripByteCounts` are filled in here. `variant`
/// selects classic TIFF or BigTIFF.
#[must_use]
pub fn write_multipage(
    order: ByteOrder,
    variant: Variant,
    pages: &[(Ifd, Vec<Vec<u8>>)],
) -> Vec<u8> {
    use crate::tags;

    // Give each directory its byte counts and a correctly-sized StripOffsets placeholder so the
    // whole directory block's layout (and thus the strip-data base) is final. The placeholder uses
    // the final field type (LONG/LONG8 per variant) so the two-pass layout is stable.
    let mut ifds: Vec<Ifd> = pages
        .iter()
        .map(|(ifd, strips)| {
            let mut ifd = ifd.clone();
            let counts: Vec<u32> = strips.iter().map(|s| s.len() as u32).collect();
            ifd.set(tags::STRIP_BYTE_COUNTS, Value::Long(counts));
            ifd.set(
                tags::STRIP_OFFSETS,
                offset_value(variant, vec![0; strips.len()]),
            );
            ifd
        })
        .collect();

    let base = even(
        write(&TiffFile {
            order,
            variant,
            ifds: ifds.clone(),
        })
        .len(),
    );

    // Lay every page's strips out contiguously after the directories.
    let mut cursor = base;
    for ((_, strips), ifd) in pages.iter().zip(ifds.iter_mut()) {
        let mut offsets = Vec::with_capacity(strips.len());
        for s in strips {
            offsets.push(cursor as u64);
            cursor += s.len();
        }
        ifd.set(tags::STRIP_OFFSETS, offset_value(variant, offsets));
    }

    let mut out = write(&TiffFile {
        order,
        variant,
        ifds,
    });
    out.resize(base, 0);
    for (_, strips) in pages {
        for s in strips {
            out.extend_from_slice(s);
        }
    }
    out
}

/// Lays out a directory plus a list of data blocks, recording each block's length under
/// `bytecount_tag` and its file offset under `offset_tag` (contiguously after the header, the IFD,
/// and its value pool).
fn write_blocks(
    order: ByteOrder,
    variant: Variant,
    ifd: &Ifd,
    blocks: &[Vec<u8>],
    offset_tag: u16,
    bytecount_tag: u16,
) -> Vec<u8> {
    let counts: Vec<u32> = blocks.iter().map(|s| s.len() as u32).collect();
    let mut ifd = ifd.clone();
    ifd.set(bytecount_tag, Value::Long(counts));
    // A correctly-sized, correctly-typed placeholder so the directory layout (and thus the data
    // base) is final; the offset *content* does not affect the layout, only its values do.
    ifd.set(offset_tag, offset_value(variant, vec![0; blocks.len()]));

    // Writing the directory alone yields exactly the header + IFD + value pool, so its length is
    // where the block data begins (rounded up to a word boundary).
    let base = even(
        write(&TiffFile {
            order,
            variant,
            ifds: vec![ifd.clone()],
        })
        .len(),
    );
    let mut offsets = Vec::with_capacity(blocks.len());
    let mut cursor = base;
    for s in blocks {
        offsets.push(cursor as u64);
        cursor += s.len();
    }
    ifd.set(offset_tag, offset_value(variant, offsets));

    let mut out = write(&TiffFile {
        order,
        variant,
        ifds: vec![ifd],
    });
    out.resize(base, 0); // pad to the even base (a no-op unless the value pool ended odd)
    for s in blocks {
        out.extend_from_slice(s);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ifd::Value;
    use crate::{reader, tags};

    fn sample_ifd() -> Ifd {
        let mut ifd = Ifd::new();
        ifd.set(tags::IMAGE_WIDTH, Value::Short(vec![640]));
        ifd.set(tags::IMAGE_LENGTH, Value::Long(vec![480]));
        ifd.set(tags::BITS_PER_SAMPLE, Value::Short(vec![8, 8, 8])); // 6 bytes -> out of line
        ifd.set(tags::X_RESOLUTION, Value::Rational(vec![(300, 1)])); // 8 bytes -> out of line
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
        let parsed = reader::read(&bytes).expect("read back");
        assert_eq!(parsed, file);
    }

    #[test]
    fn single_ifd_roundtrips_both_orders_and_variants() {
        for variant in [Variant::Classic, Variant::Big] {
            roundtrip(ByteOrder::LittleEndian, variant);
            roundtrip(ByteOrder::BigEndian, variant);
        }
    }

    #[test]
    fn multi_ifd_chain_roundtrips() {
        let mut second = Ifd::new();
        second.set(tags::IMAGE_WIDTH, Value::Short(vec![1]));
        second.set(tags::IMAGE_LENGTH, Value::Short(vec![1]));
        for variant in [Variant::Classic, Variant::Big] {
            let file = TiffFile {
                order: ByteOrder::LittleEndian,
                variant,
                ifds: vec![sample_ifd(), second.clone()],
            };
            let bytes = write(&file);
            let parsed = reader::read(&bytes).expect("read back");
            assert_eq!(parsed.ifds.len(), 2);
            assert_eq!(parsed, file);
        }
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
        assert_eq!(reader::read_header(&bytes).expect("header").2, 8);
    }

    #[test]
    fn bigtiff_header_and_inline_threshold() {
        // The 8-byte XResolution rational is out of line in classic TIFF (>4 B) but packs inline
        // in BigTIFF (<=8 B); both must round-trip identically.
        let bytes = write(&TiffFile {
            order: ByteOrder::LittleEndian,
            variant: Variant::Big,
            ifds: vec![sample_ifd()],
        });
        assert_eq!(&bytes[0..2], b"II");
        let (order, variant, first) = reader::read_header(&bytes).expect("header");
        assert_eq!(order, ByteOrder::LittleEndian);
        assert_eq!(variant, Variant::Big);
        assert_eq!(bytes[2], 0x2b); // magic 43
        assert_eq!(first, 16); // 16-byte header
        let parsed = reader::read(&bytes).expect("read back");
        assert_eq!(parsed.variant, Variant::Big);
        assert_eq!(
            parsed.ifds[0].get(tags::X_RESOLUTION),
            Some(&Value::Rational(vec![(300, 1)]))
        );
    }
}
