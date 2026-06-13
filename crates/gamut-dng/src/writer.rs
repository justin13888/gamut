//! Codec-level serialisation: laying the preview and raw image data out around the DNG's IFD tree.
//!
//! The structural layout — the byte-order header, the IFD *tree* (IFD 0 plus its raw sub-IFD), and
//! the two-pass offset layout of out-of-line values — is [`gamut_ifd::write`]. This module composes
//! that primitive with the codec's pixel data: it writes the directory tree once with correctly
//! sized placeholder offsets to learn where the value pool ends, places each image's strips after
//! it, fills in the `StripOffsets`/`StripByteCounts` (or tile equivalents), and writes again. The
//! re-write is byte-stable because only the *values* of the offset fields change, not their sizes.

use gamut_ifd::{ByteOrder, Ifd, TiffFile, Value, Variant, write};

use crate::tags;

/// Rounds `n` up to the next even (word) boundary, matching [`gamut_ifd::write`]'s value-pool
/// alignment so block data begins on a word boundary.
fn even(n: usize) -> usize {
    n + (n & 1)
}

/// Builds an offset-array field of the right type for `variant`: `LONG` classic, `LONG8` BigTIFF.
fn offset_value(variant: Variant, offsets: Vec<u64>) -> Value {
    match variant {
        Variant::Classic => Value::Long(offsets.iter().map(|&o| o as u32).collect()),
        Variant::Big => Value::Long8(offsets),
    }
}

/// One image's data blocks (strips or tiles) plus the tags that reference them.
pub(crate) struct ImageBlocks {
    /// The offset tag — [`tags::STRIP_OFFSETS`] or [`tags::TILE_OFFSETS`].
    pub offset_tag: u16,
    /// The byte-count tag — [`tags::STRIP_BYTE_COUNTS`] or [`tags::TILE_BYTE_COUNTS`].
    pub bytecount_tag: u16,
    /// The data blocks (already compressed, if applicable), in file order.
    pub blocks: Vec<Vec<u8>>,
}

impl ImageBlocks {
    /// Stores the byte counts and a correctly-typed placeholder offset array into `ifd`, so the
    /// directory layout is final before the real offsets are known.
    fn install_placeholders(&self, ifd: &mut Ifd, variant: Variant) {
        let counts: Vec<u32> = self.blocks.iter().map(|b| b.len() as u32).collect();
        ifd.set(self.bytecount_tag, Value::Long(counts));
        ifd.set(
            self.offset_tag,
            offset_value(variant, vec![0; self.blocks.len()]),
        );
    }
}

/// Assigns each block in `image` a file offset starting at `*cursor`, advancing it past the data,
/// and records the offsets under the image's offset tag in `ifd`.
fn place_blocks(cursor: &mut usize, image: &ImageBlocks, ifd: &mut Ifd, variant: Variant) {
    let mut offsets = Vec::with_capacity(image.blocks.len());
    for block in &image.blocks {
        offsets.push(*cursor as u64);
        *cursor += block.len();
    }
    ifd.set(image.offset_tag, offset_value(variant, offsets));
}

/// Serialises a DNG whose IFD 0 carries `preview` image data and whose single raw sub-IFD
/// (`SubIFDs`, [`tags::SUB_IFDS`]) carries `raw` image data.
///
/// `ifd0` and `raw_ifd` supply every field *except* the offset/byte-count pair, which this fills
/// in. The preview data is laid out first, then the raw data, both after the directory tree.
#[must_use]
pub(crate) fn write_cfa_dng(
    order: ByteOrder,
    variant: Variant,
    mut ifd0: Ifd,
    preview: &ImageBlocks,
    mut raw_ifd: Ifd,
    raw: &ImageBlocks,
) -> Vec<u8> {
    // Install correctly-sized placeholders so the directory tree's byte layout is final.
    preview.install_placeholders(&mut ifd0, variant);
    raw.install_placeholders(&mut raw_ifd, variant);
    ifd0.set_sub_ifd(tags::SUB_IFDS, vec![raw_ifd.clone()]);

    // Writing the tree alone yields header + IFDs + value pool, so its length is where the image
    // data begins (rounded up to a word boundary).
    let base = even(
        write(&TiffFile {
            order,
            variant,
            ifds: vec![ifd0.clone()],
        })
        .len(),
    );

    // Lay the preview strips out first, then the raw strips, recording the real offsets.
    let mut cursor = base;
    place_blocks(&mut cursor, preview, &mut ifd0, variant);
    place_blocks(&mut cursor, raw, &mut raw_ifd, variant);
    ifd0.set_sub_ifd(tags::SUB_IFDS, vec![raw_ifd]);

    let mut out = write(&TiffFile {
        order,
        variant,
        ifds: vec![ifd0],
    });
    out.resize(base, 0); // pad to the even base (a no-op unless the value pool ended odd)
    for block in &preview.blocks {
        out.extend_from_slice(block);
    }
    for block in &raw.blocks {
        out.extend_from_slice(block);
    }
    out
}
