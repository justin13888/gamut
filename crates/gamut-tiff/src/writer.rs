//! Serialisation of the TIFF byte-order header and the IFD chain.
//!
//! The writer lays the file out in two passes: it first computes where each IFD and each
//! out-of-line value lands (header → IFDs → value pool, every block on an even/word boundary),
//! then emits the bytes with the absolute offsets patched in. Values of four bytes or fewer are
//! packed inline in the entry, left-justified (TIFF 6.0 §2).

use crate::ifd::{ByteOrder, Ifd};
use crate::reader::TiffFile;

/// Rounds `n` up to the next even (word) boundary, as required for value offsets.
fn even(n: usize) -> usize {
    n + (n & 1)
}

/// The on-disk size of an IFD: count (2) + 12 per entry + next-offset (4).
fn ifd_size(ifd: &Ifd) -> usize {
    2 + ifd.fields().len() * 12 + 4
}

/// Serialises a TIFF file (header + IFD chain + image/value data) to bytes.
///
/// IFDs are written in order and linked through their next-IFD pointers; out-of-line values are
/// appended in a value pool after the directories.
#[must_use]
pub fn write(file: &TiffFile) -> Vec<u8> {
    let order = file.order;

    // Pass 1: place the header and IFDs, then the value pool. Record each IFD's start and each
    // out-of-line value's (offset, bytes).
    let mut cursor = even(8);
    let mut ifd_offsets = Vec::with_capacity(file.ifds.len());
    for ifd in &file.ifds {
        ifd_offsets.push(cursor as u32);
        cursor = even(cursor + ifd_size(ifd));
    }
    // Per-IFD, per-field out-of-line value offset (0 if packed inline).
    let mut value_offsets: Vec<Vec<u32>> = Vec::with_capacity(file.ifds.len());
    let mut pool: Vec<(usize, Vec<u8>)> = Vec::new();
    for ifd in &file.ifds {
        let mut offs = Vec::with_capacity(ifd.fields().len());
        for field in ifd.fields() {
            let bytes = field.value.encode(order);
            if bytes.len() <= 4 {
                offs.push(0);
            } else {
                cursor = even(cursor);
                offs.push(cursor as u32);
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
    out[2..4].copy_from_slice(&order.pack_u16(42));
    out[4..8].copy_from_slice(&order.pack_u32(ifd_offsets.first().copied().unwrap_or(0)));

    for (idx, ifd) in file.ifds.iter().enumerate() {
        let mut pos = ifd_offsets[idx] as usize;
        out[pos..pos + 2].copy_from_slice(&order.pack_u16(ifd.fields().len() as u16));
        pos += 2;
        for (field, &voff) in ifd.fields().iter().zip(&value_offsets[idx]) {
            let bytes = field.value.encode(order);
            out[pos..pos + 2].copy_from_slice(&order.pack_u16(field.tag));
            out[pos + 2..pos + 4].copy_from_slice(&order.pack_u16(field.value.field_type().code()));
            out[pos + 4..pos + 8].copy_from_slice(&order.pack_u32(field.value.count() as u32));
            if bytes.len() <= 4 {
                // Inline, left-justified: low bytes hold the value, remainder is zero.
                out[pos + 8..pos + 8 + bytes.len()].copy_from_slice(&bytes);
            } else {
                out[pos + 8..pos + 12].copy_from_slice(&order.pack_u32(voff));
            }
            pos += 12;
        }
        let next = file.ifds.get(idx + 1).map_or(0, |_| ifd_offsets[idx + 1]);
        out[pos..pos + 4].copy_from_slice(&order.pack_u32(next));
    }

    for (offset, bytes) in pool {
        out[offset..offset + bytes.len()].copy_from_slice(&bytes);
    }
    out
}

/// Serialises a single-IFD TIFF image: the supplied directory plus its strip data.
///
/// `StripByteCounts` is set from the strip lengths and `StripOffsets` from where the strips land —
/// contiguously after the header, the IFD, and its value pool. The caller supplies the rest of the
/// directory (dimensions, photometric, compression, …). The strips are written verbatim, so the
/// caller is responsible for any per-strip compression.
#[must_use]
pub fn write_image(order: ByteOrder, ifd: &Ifd, strips: &[Vec<u8>]) -> Vec<u8> {
    use crate::ifd::Value;
    use crate::tags;

    let counts: Vec<u32> = strips.iter().map(|s| s.len() as u32).collect();
    let mut ifd = ifd.clone();
    ifd.set(tags::STRIP_BYTE_COUNTS, Value::Long(counts));
    // A correctly-sized placeholder so the directory layout (and thus the strip base) is final;
    // the StripOffsets *content* does not affect the layout, only its values do.
    ifd.set(tags::STRIP_OFFSETS, Value::Long(vec![0; strips.len()]));

    // Writing the directory alone yields exactly the header + IFD + value pool, so its length is
    // where the strip data begins (rounded up to a word boundary).
    let base = even(
        write(&TiffFile {
            order,
            ifds: vec![ifd.clone()],
        })
        .len(),
    );
    let mut offsets = Vec::with_capacity(strips.len());
    let mut cursor = base;
    for s in strips {
        offsets.push(cursor as u32);
        cursor += s.len();
    }
    ifd.set(tags::STRIP_OFFSETS, Value::Long(offsets));

    let mut out = write(&TiffFile {
        order,
        ifds: vec![ifd],
    });
    out.resize(base, 0); // pad to the even strip base (a no-op unless the value pool ended odd)
    for s in strips {
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

    fn roundtrip(order: ByteOrder) {
        let file = TiffFile {
            order,
            ifds: vec![sample_ifd()],
        };
        let bytes = write(&file);
        let parsed = reader::read(&bytes).expect("read back");
        assert_eq!(parsed, file);
    }

    #[test]
    fn single_ifd_roundtrips_both_orders() {
        roundtrip(ByteOrder::LittleEndian);
        roundtrip(ByteOrder::BigEndian);
    }

    #[test]
    fn multi_ifd_chain_roundtrips() {
        let mut second = Ifd::new();
        second.set(tags::IMAGE_WIDTH, Value::Short(vec![1]));
        second.set(tags::IMAGE_LENGTH, Value::Short(vec![1]));
        let file = TiffFile {
            order: ByteOrder::LittleEndian,
            ifds: vec![sample_ifd(), second],
        };
        let bytes = write(&file);
        let parsed = reader::read(&bytes).expect("read back");
        assert_eq!(parsed.ifds.len(), 2);
        assert_eq!(parsed, file);
    }

    #[test]
    fn value_offsets_are_even() {
        // BitsPerSample (6 bytes) forces an out-of-line value; its offset must be word-aligned.
        let bytes = write(&TiffFile {
            order: ByteOrder::LittleEndian,
            ifds: vec![sample_ifd()],
        });
        // Header magic and first-IFD offset are well-formed.
        assert_eq!(&bytes[0..2], b"II");
        assert_eq!(reader::read_header(&bytes).expect("header").1, 8);
    }
}
