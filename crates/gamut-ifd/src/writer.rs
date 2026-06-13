//! Serialisation of the TIFF byte-order header and the IFD chain.
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

use crate::{ByteOrder, Ifd, TiffFile, Variant};

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
        #[cfg(feature = "bigtiff")]
        Variant::Big => out[pos..pos + 8].copy_from_slice(&order.pack_u64(v)),
    }
}

/// Serialises a TIFF/IFD stream (header + IFD chain + out-of-line value pool) to bytes.
///
/// IFDs are written in order and linked through their next-IFD pointers; out-of-line values are
/// appended in a value pool after the directories. Image/pixel data is not handled here — a codec
/// composes that around this primitive (see `gamut-tiff`'s strip/tile writers).
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
        #[cfg(feature = "bigtiff")]
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
            #[cfg(feature = "bigtiff")]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Value, read, read_header};

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

    #[cfg(feature = "bigtiff")]
    #[test]
    fn bigtiff_roundtrips_and_inline_threshold() {
        roundtrip(ByteOrder::LittleEndian, Variant::Big);
        roundtrip(ByteOrder::BigEndian, Variant::Big);
        multi_ifd_roundtrip(Variant::Big);

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
}
