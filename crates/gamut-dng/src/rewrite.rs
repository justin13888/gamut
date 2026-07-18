//! The preserving rewrite: read a DNG losslessly, edit it surgically, write it back with
//! **nothing dropped** (issue #263).
//!
//! The typed codec ([`crate::DngEncoder`] / [`crate::DngDecoder`]) is deliberately lossy: it
//! models the DNG spec's structures and rebuilds files from that model, so unmodeled and vendor
//! material does not survive a decode → encode cycle. [`DngRewrite`] is the preservation path:
//! it parses the **whole** IFD tree on the lossless `gamut-ifd` model (unknown/vendor tags,
//! unknown field types — everything survives as data), exposes the tree for surgical edits, and
//! re-serialises it carrying every pixel payload **byte-for-byte** (strips/tiles are copied,
//! never re-encoded) and every tag value byte-exactly.
//!
//! The `MakerNote` blob gets special care: vendor notes commonly encode internal offsets
//! relative to the enclosing TIFF header, so relocating the blob makes them stale. The rewrite
//! **pins** the note at its original absolute offset whenever the new layout permits
//! ([`MakerNotePreservation`] reports the outcome).

use gamut_core::{Error, Result};
use gamut_ifd::{
    ByteOrder, Ifd, IfdReader, TiffFile, Value, Variant, WriteOptions, align_word,
    tags as ifd_tags, write_with,
};

use crate::decoder::DngDecoder;
use crate::tags;

/// What happened to the `MakerNote` byte range on a rewrite.
///
/// `#[non_exhaustive]`: further outcomes (e.g. vendor-aware rebasing) may be added without a
/// breaking change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MakerNotePreservation {
    /// The file carries no (out-of-line) maker note.
    Absent,
    /// The note's bytes sit at their **original absolute offset** — vendor-internal offsets
    /// stay valid.
    Pinned,
    /// The note's bytes are intact but relocated (the original offset now collides with the
    /// new directory layout); vendor-internal absolute offsets may be stale.
    Relocated,
}

/// The output of [`DngRewrite::write`]: the serialised stream and what happened to the
/// offset-sensitive material.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RewrittenDng {
    /// The rewritten DNG stream.
    pub bytes: Vec<u8>,
    /// What happened to the `MakerNote` byte range.
    pub maker_note: MakerNotePreservation,
}

/// A DNG opened for a preserving rewrite: the full IFD tree on the lossless model, plus the
/// original bytes (the source of the pixel payloads).
#[derive(Debug, Clone)]
pub struct DngRewrite {
    data: Vec<u8>,
    order: ByteOrder,
    variant: Variant,
    file: TiffFile,
    /// The original absolute offset of the (out-of-line) `MakerNote` value, if any.
    maker_note_at: Option<u64>,
}

impl DngRewrite {
    /// Opens `data` for rewriting: parses the whole sub-IFD tree losslessly and records the
    /// original absolute position of the `MakerNote` value.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if the container or its sub-IFD tree is unreadable — a
    /// preserving rewrite refuses to guess — and [`Error::Unsupported`] if the file carries
    /// `ExtraCameraProfiles` (embedded camera-profile streams; carrying them through a rewrite
    /// is deferred).
    pub fn open(data: &[u8]) -> Result<Self> {
        let (order, variant, _) = gamut_ifd::read_header(data)?;
        let file = gamut_ifd::read_tree(data, ifd_tags::STANDARD_POINTER_TAGS)?;
        for ifd in &file.ifds {
            if ifd.get(tags::EXTRA_CAMERA_PROFILES).is_some() {
                return Err(Error::Unsupported(
                    "DNG: rewriting a file with ExtraCameraProfiles is not supported yet",
                ));
            }
        }
        let maker_note_at = find_maker_note_offset(data)?;
        Ok(Self {
            data: data.to_vec(),
            order,
            variant,
            file,
            maker_note_at,
        })
    }

    /// The parsed tree: every page, sub-IFD, and tag — including unknown/vendor material —
    /// exactly as read.
    #[must_use]
    pub fn file(&self) -> &TiffFile {
        &self.file
    }

    /// The stream's byte order (preserved through the rewrite).
    #[must_use]
    pub fn order(&self) -> ByteOrder {
        self.order
    }

    /// The container variant (classic or BigTIFF; preserved through the rewrite).
    #[must_use]
    pub fn variant(&self) -> Variant {
        self.variant
    }

    /// The parsed tree, for surgical edits. Pixel payloads are located through each image IFD's
    /// own strip/tile/JPEG-interchange tags at [`write`](Self::write) time, so edit those tags
    /// only if the bytes they point at (in the *original* stream) are what you mean.
    pub fn file_mut(&mut self) -> &mut TiffFile {
        &mut self.file
    }

    /// Decodes the **original** stream's typed view (raw image, profile, metadata) — a
    /// convenience for reading; edits made through [`file_mut`](Self::file_mut) are not
    /// reflected here.
    ///
    /// # Errors
    ///
    /// Returns [`gamut_core::Error`] under the same conditions as [`DngDecoder::decode`].
    pub fn decode(&self) -> Result<crate::DecodedDng> {
        DngDecoder::new().decode(&self.data)
    }

    /// Serialises the (possibly edited) tree, preserving everything: every tag value
    /// byte-exactly (unknown field types verbatim), every strip/tile/embedded-JPEG payload
    /// copied byte-for-byte from the original stream, and the maker note pinned at its original
    /// absolute offset when the new layout permits.
    ///
    /// Declared dead space (`FreeOffsets`/`FreeByteCounts`) is dropped — the one intentional
    /// omission, since those tags name explicitly-dead bytes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if a payload range lies outside the original stream, an
    /// offset/count pair is incoherent, or the layout is unrepresentable (classic width
    /// overflow).
    pub fn write(&self) -> Result<RewrittenDng> {
        let mut tree = self.file.clone();

        // Pass A: collect every payload (bytes copied from the original stream, in DFS order)
        // and replace the offset arrays with correctly-sized placeholders so the directory
        // layout is final.
        let mut payloads: Vec<Payload> = Vec::new();
        for ifd in &mut tree.ifds {
            collect_payloads(ifd, &self.data, self.variant, &mut payloads)?;
        }

        // The maker-note pin: only meaningful if the note existed out of line originally and
        // still exists in the (possibly edited) tree.
        let pin_at = self
            .maker_note_at
            .filter(|_| tree_has_tag(&tree, ifd_tags::MAKER_NOTE));
        let opts = |pin: Option<u64>| {
            let mut o = WriteOptions::default();
            if let Some(at) = pin {
                o = o.pin(ifd_tags::MAKER_NOTE, at);
            }
            o
        };

        // Probe pass: the layout is a pure function of the structure (placeholders are final
        // sized), so measuring once fixes where the payload region begins. If pinning is
        // unsatisfiable in the new layout, fall back to relocating the note.
        let mut maker_note = if pin_at.is_some() {
            MakerNotePreservation::Pinned
        } else if tree_has_tag(&tree, ifd_tags::MAKER_NOTE) {
            MakerNotePreservation::Relocated
        } else {
            MakerNotePreservation::Absent
        };
        let mut pin = pin_at;
        let probe = match write_with(&tree, &opts(pin)) {
            Ok((bytes, _)) => bytes,
            Err(_) if pin.is_some() => {
                pin = None;
                maker_note = MakerNotePreservation::Relocated;
                write_with(&tree, &opts(None))?.0
            }
            Err(e) => return Err(e),
        };

        // Assign the payload offsets after the probe layout, word-aligned, in DFS order.
        let base = align_word(probe.len() as u64);
        let mut cursor = base;
        let mut placed: Vec<Vec<u64>> = Vec::with_capacity(payloads.len());
        for payload in &payloads {
            let mut offsets = Vec::with_capacity(payload.blocks.len());
            for block in &payload.blocks {
                cursor = align_word(cursor);
                offsets.push(cursor);
                cursor += block.len() as u64;
            }
            placed.push(offsets);
        }

        // Pass B: patch the real offsets into the tree (same value widths as the placeholders,
        // so the layout is byte-identical to the probe), emit, and append the payloads.
        let mut cursor_sets = placed.iter();
        for ifd in &mut tree.ifds {
            patch_payload_offsets(ifd, self.variant, &mut cursor_sets)?;
        }
        let (mut bytes, _map) = write_with(&tree, &opts(pin))?;
        debug_assert_eq!(bytes.len(), probe.len(), "structural determinism");
        let base =
            usize::try_from(base).map_err(|_| Error::InvalidInput("DNG: layout overflows"))?;
        bytes.resize(base, 0);
        for (payload, offsets) in payloads.iter().zip(&placed) {
            for (block, &offset) in payload.blocks.iter().zip(offsets) {
                let offset = usize::try_from(offset)
                    .map_err(|_| Error::InvalidInput("DNG: layout overflows"))?;
                bytes.resize(offset, 0); // the ≤1 byte of word-alignment padding
                bytes.extend_from_slice(block);
            }
        }
        if stream_overflows(self.variant, bytes.len() as u64) {
            return Err(Error::InvalidInput(
                "DNG: layout exceeds the 4 GiB classic-TIFF offset limit",
            ));
        }
        Ok(RewrittenDng { bytes, maker_note })
    }
}

/// Whether the final stream (directories + appended payloads) outgrew classic TIFF's 32-bit
/// offsets. A pure predicate so the 4 GiB boundary is unit-testable without a 4 GiB allocation
/// (mirroring `gamut_ifd`'s `layout_overflows`).
fn stream_overflows(variant: Variant, len: u64) -> bool {
    variant == Variant::Classic && len > u64::from(u32::MAX)
}

/// One directory's payload blocks (strips, tiles, or an embedded JPEG), copied verbatim.
struct Payload {
    blocks: Vec<Vec<u8>>,
}

/// The `(offset tag, byte-count tag)` pairs that locate payload bytes, in the order both walk
/// passes visit them.
const PAYLOAD_PAIRS: &[(u16, u16)] = &[
    (tags::STRIP_OFFSETS, tags::STRIP_BYTE_COUNTS),
    (tags::TILE_OFFSETS, tags::TILE_BYTE_COUNTS),
    (
        ifd_tags::JPEG_INTERCHANGE_FORMAT,
        ifd_tags::JPEG_INTERCHANGE_FORMAT_LENGTH,
    ),
];

/// DFS pass A over `ifd` and its sub-IFDs: copies every payload out of `data`, swaps the offset
/// arrays for correctly-sized placeholders, and drops declared dead space.
fn collect_payloads(
    ifd: &mut Ifd,
    data: &[u8],
    variant: Variant,
    payloads: &mut Vec<Payload>,
) -> Result<()> {
    for &(off_tag, cnt_tag) in PAYLOAD_PAIRS {
        let Some(offsets) = ifd.get_u64_vec(off_tag) else {
            continue;
        };
        let counts = ifd.get_u64_vec(cnt_tag).ok_or(Error::InvalidInput(
            "DNG: payload offsets without matching byte counts",
        ))?;
        if offsets.len() != counts.len() {
            return Err(Error::InvalidInput(
                "DNG: payload offset/byte-count length mismatch",
            ));
        }
        let mut blocks = Vec::with_capacity(offsets.len());
        for (&offset, &count) in offsets.iter().zip(&counts) {
            let start = usize::try_from(offset)
                .map_err(|_| Error::InvalidInput("DNG: payload offset out of bounds"))?;
            let len = usize::try_from(count)
                .map_err(|_| Error::InvalidInput("DNG: payload out of bounds"))?;
            let block = start
                .checked_add(len)
                .and_then(|end| data.get(start..end))
                .ok_or(Error::InvalidInput("DNG: payload out of bounds"))?;
            blocks.push(block.to_vec());
        }
        // A placeholder of the same element count and offset width keeps the layout final.
        ifd.set(
            off_tag,
            Value::offset_array(variant, &vec![0u64; blocks.len()])?,
        );
        payloads.push(Payload { blocks });
    }
    // Declared dead space is dropped, not carried: the tags name explicitly-dead bytes.
    ifd.remove(ifd_tags::FREE_OFFSETS);
    ifd.remove(ifd_tags::FREE_BYTE_COUNTS);
    for group in ifd.sub_ifds_mut() {
        for child in &mut group.ifds {
            collect_payloads(child, data, variant, payloads)?;
        }
    }
    Ok(())
}

/// DFS pass B, mirroring pass A's visit order exactly: writes the placed offsets over the
/// placeholder arrays.
fn patch_payload_offsets<'a>(
    ifd: &mut Ifd,
    variant: Variant,
    placed: &mut impl Iterator<Item = &'a Vec<u64>>,
) -> Result<()> {
    for &(off_tag, _) in PAYLOAD_PAIRS {
        if ifd.get(off_tag).is_none() {
            continue;
        }
        let offsets = placed
            .next()
            .ok_or(Error::InvalidInput("DNG: payload bookkeeping out of sync"))?;
        ifd.set(off_tag, Value::offset_array(variant, offsets)?);
    }
    for group in ifd.sub_ifds_mut() {
        for child in &mut group.ifds {
            patch_payload_offsets(child, variant, placed)?;
        }
    }
    Ok(())
}

/// Whether any directory in the tree carries `tag` as a field.
fn tree_has_tag(file: &TiffFile, tag: u16) -> bool {
    fn ifd_has(ifd: &Ifd, tag: u16) -> bool {
        ifd.get(tag).is_some()
            || ifd
                .sub_ifds()
                .iter()
                .flat_map(|g| &g.ifds)
                .any(|child| ifd_has(child, tag))
    }
    file.ifds.iter().any(|ifd| ifd_has(ifd, tag))
}

/// Finds the original absolute offset of the Exif sub-IFD's out-of-line `MakerNote` value
/// (`None` if absent or inline).
fn find_maker_note_offset(data: &[u8]) -> Result<Option<u64>> {
    let mut reader = IfdReader::open(data)?;
    let ifd0 = reader.read_ifd(reader.first_ifd_offset())?;
    let Some(exif_entry) = ifd0.entry(ifd_tags::EXIF_IFD) else {
        return Ok(None);
    };
    let Ok(exif_off) = reader.value(exif_entry).map(|v| v.as_u64()) else {
        return Ok(None);
    };
    let Some(exif_off) = exif_off else {
        return Ok(None);
    };
    let Ok(exif) = reader.read_ifd(exif_off) else {
        return Ok(None);
    };
    let Some(note) = exif.entry(ifd_tags::MAKER_NOTE) else {
        return Ok(None);
    };
    Ok(reader.value_offset(note))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 4 GiB boundary, on the pure predicate: the largest classic length is representable,
    /// one past it is not, and BigTIFF never overflows.
    #[test]
    fn stream_overflow_boundary() {
        assert!(!stream_overflows(Variant::Classic, 100));
        assert!(!stream_overflows(Variant::Classic, u64::from(u32::MAX)));
        assert!(stream_overflows(Variant::Classic, u64::from(u32::MAX) + 1));
        assert!(!stream_overflows(Variant::Big, u64::from(u32::MAX) + 1));
    }
}
