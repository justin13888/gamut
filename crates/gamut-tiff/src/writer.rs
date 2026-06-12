//! Codec-level serialisation: laying image (strip/tile) data out around an IFD.
//!
//! The structural serialisation — the byte-order header, the IFD chain, and the two-pass offset
//! layout of out-of-line values — is [`gamut_ifd::write`]. This module composes that primitive with
//! the codec's pixel data: it sizes the directory, places each strip/tile contiguously after it, and
//! fills in the `StripOffsets`/`StripByteCounts` (or `TileOffsets`/`TileByteCounts`) that point at
//! them. `variant` selects classic TIFF or BigTIFF throughout.

use gamut_ifd::{ByteOrder, Ifd, TiffFile, Value, Variant, write};

/// Rounds `n` up to the next even (word) boundary, matching the value-pool alignment
/// [`gamut_ifd::write`] uses, so block data begins on a word boundary.
fn even(n: usize) -> usize {
    n + (n & 1)
}

/// Builds a strip/tile-offset field value of the right type for `variant`: `LONG` for classic
/// TIFF, `LONG8` for BigTIFF (whose offsets may exceed 4 GiB).
fn offset_value(variant: Variant, offsets: Vec<u64>) -> Value {
    match variant {
        Variant::Classic => Value::Long(offsets.iter().map(|&o| o as u32).collect()),
        Variant::Big => Value::Long8(offsets),
    }
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
