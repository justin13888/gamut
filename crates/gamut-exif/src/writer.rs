//! Writing an [`Exif`] back to a valid EXIF blob.
//!
//! The writer hands the whole IFD tree to [`gamut_ifd::write`], whose two-pass offset layout does
//! the hard part: it places every directory, appends one value pool, and synthesises each sub-IFD
//! pointer field with the child's patched offset. This crate's job is only to shape the tree —
//! attach the Exif/GPS/Interop sub-IFDs under their pointer tags and chain the thumbnail as the 1st
//! IFD — so the round-trip `parse → write → parse` reproduces the directories with the source byte
//! order preserved.

use gamut_ifd::{ByteOrder, Ifd, TiffFile, Value, Variant, write};

use crate::error::Result;
use crate::exif::{
    EXIF_IFD_POINTER, Exif, GPS_IFD_POINTER, INTEROP_IFD_POINTER, MARKER, without_tags,
};
use crate::tag::ExifTag;
use crate::thumbnail::Thumbnail;

/// Serialises an [`Exif`] back to an EXIF blob, with options for the marker and byte order.
///
/// By default it emits the `Exif\0\0` marker (as a JPEG `APP1` segment needs) and preserves the
/// [`Exif`]'s byte order.
#[derive(Debug, Clone)]
pub struct ExifWriter {
    marker: bool,
    byte_order: Option<ByteOrder>,
}

impl Default for ExifWriter {
    fn default() -> Self {
        Self {
            marker: true,
            byte_order: None,
        }
    }
}

impl ExifWriter {
    /// A writer with default options (emit the marker; keep the source byte order).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether to prefix the output with the `Exif\0\0` marker.
    ///
    /// Pass `false` for a bare TIFF stream — what the PNG `eXIf` and WebP `EXIF` chunks carry.
    #[must_use]
    pub fn marker(mut self, yes: bool) -> Self {
        self.marker = yes;
        self
    }

    /// Overrides the byte order the stream is written in (default: the [`Exif`]'s own order).
    #[must_use]
    pub fn byte_order(mut self, order: ByteOrder) -> Self {
        self.byte_order = Some(order);
        self
    }

    /// Serialises `exif` to bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ExifError::Ifd`](crate::ExifError::Ifd) if the model is not representable in
    /// classic-TIFF widths (a directory of more than `u16::MAX` entries, or a stream past the
    /// 4 GiB offset limit) — far beyond what any EXIF carrier accepts.
    pub fn write(&self, exif: &Exif) -> Result<Vec<u8>> {
        let order = self.byte_order.unwrap_or_else(|| exif.byte_order());
        let image = build_image(exif);
        let tiff = write_with_thumbnail(order, image, exif.thumbnail())?;

        Ok(if self.marker {
            let mut out = MARKER.to_vec();
            out.extend(tiff);
            out
        } else {
            tiff
        })
    }
}

/// Rebuilds the 0th IFD: drops any hand-set pointer tags and re-attaches the Exif/GPS sub-IFDs (the
/// Exif sub-IFD itself nesting the Interop sub-IFD) so [`gamut_ifd::write`] synthesises correct
/// pointer offsets. An Interop directory implies an Exif directory to hold its pointer.
fn build_image(exif: &Exif) -> Ifd {
    let exif_sub = if exif.exif_ifd().is_some() || exif.interop_ifd().is_some() {
        let mut e = exif
            .exif_ifd()
            .map_or_else(Ifd::new, |e| without_tags(e, &[INTEROP_IFD_POINTER]));
        if let Some(interop) = exif.interop_ifd() {
            e.set_sub_ifd(INTEROP_IFD_POINTER, vec![interop.clone()]);
        }
        Some(e)
    } else {
        None
    };

    let mut image = without_tags(exif.image(), &[EXIF_IFD_POINTER, GPS_IFD_POINTER]);
    if let Some(e) = exif_sub {
        image.set_sub_ifd(EXIF_IFD_POINTER, vec![e]);
    }
    if let Some(gps) = exif.gps_ifd() {
        image.set_sub_ifd(GPS_IFD_POINTER, vec![gps.clone()]);
    }
    image
}

/// Serialises the TIFF stream, chaining the thumbnail as the 1st IFD.
///
/// A JPEG thumbnail is re-embedded with a deterministic double-write: lay out the directories with a
/// placeholder `JPEGInterchangeFormat` offset to learn where the bytes will land, patch the offset
/// (an inline `LONG`, so the layout and total length are unchanged), then append the JPEG. An
/// uncompressed strip thumbnail's directory is preserved but its pixel bytes are **not** re-embedded
/// (a documented v1 limitation).
fn write_with_thumbnail(
    order: ByteOrder,
    image: Ifd,
    thumbnail: Option<&Thumbnail>,
) -> Result<Vec<u8>> {
    let Some(thumb) = thumbnail else {
        return Ok(write(&tiff_file(order, vec![image]))?);
    };
    let Some(jpeg) = thumb.jpeg() else {
        return Ok(write(&tiff_file(order, vec![image, thumb.ifd().clone()]))?);
    };

    let mut thumb_ifd = thumb.ifd().clone();
    thumb_ifd.set(
        ExifTag::JpegInterchangeFormatLength.tag_id(),
        Value::Long(vec![jpeg.len() as u32]),
    );
    thumb_ifd.set(
        ExifTag::JpegInterchangeFormat.tag_id(),
        Value::Long(vec![0]),
    );

    // Pass 1: lay out the directories to learn where the JPEG will start (word-aligned).
    let planned = write(&tiff_file(order, vec![image.clone(), thumb_ifd.clone()]))?;
    let jpeg_offset = even(planned.len());

    // Pass 2: patch the now-known offset. Changing an inline LONG moves nothing, so the byte length
    // is identical to pass 1 and `jpeg_offset` still points just past the directories.
    thumb_ifd.set(
        ExifTag::JpegInterchangeFormat.tag_id(),
        Value::Long(vec![jpeg_offset as u32]),
    );
    let mut bytes = write(&tiff_file(order, vec![image, thumb_ifd]))?;
    debug_assert_eq!(jpeg_offset, even(bytes.len()));
    bytes.resize(jpeg_offset, 0);
    bytes.extend_from_slice(jpeg);
    Ok(bytes)
}

/// A classic-TIFF [`TiffFile`] in `order`.
fn tiff_file(order: ByteOrder, ifds: Vec<Ifd>) -> TiffFile {
    TiffFile {
        order,
        variant: Variant::Classic,
        ifds,
    }
}

/// Rounds `n` up to the next even (word) boundary, matching [`gamut_ifd::write`]'s value alignment.
fn even(n: usize) -> usize {
    n + (n & 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExifTag, IfdKind, Value};

    /// A model with tags spread across the 0th IFD, Exif, GPS and Interop sub-IFDs, plus a
    /// thumbnail directory — so the round-trip exercises the whole pointer tree.
    fn sample(order: ByteOrder) -> Exif {
        let mut exif = Exif::new(order);
        exif.set_tag(ExifTag::Make, Value::Ascii("Fujifilm".into()));
        exif.set_tag(ExifTag::Model, Value::Ascii("X-T5".into()));
        exif.set_tag(ExifTag::Orientation, Value::Short(vec![1]));
        exif.set_tag(ExifTag::FNumber, Value::Rational(vec![(20, 10)]));
        exif.set_tag(ExifTag::ExposureTime, Value::Rational(vec![(1, 250)]));
        exif.set_tag(ExifTag::PhotographicSensitivity, Value::Short(vec![160]));
        exif.set_tag(ExifTag::ExifVersion, Value::Undefined(b"0300".to_vec()));
        // Exif 3.0 UTF-8 text must survive the round-trip.
        exif.set_tag(ExifTag::LensModel, Value::Utf8("XF16-80mm ƒ4".into()));
        exif.set_tag(ExifTag::GpsVersionId, Value::Byte(vec![2, 3, 0, 0]));
        exif.set_tag(ExifTag::GpsLatitudeRef, Value::Ascii("N".into()));
        exif.set_tag(
            ExifTag::GpsLatitude,
            Value::Rational(vec![(48, 1), (51, 1), (0, 1)]),
        );
        exif.set_tag(ExifTag::InteroperabilityIndex, Value::Ascii("R98".into()));
        exif
    }

    fn assert_round_trips(order: ByteOrder) {
        let original = sample(order);
        let bytes = original.to_bytes().expect("write");
        let parsed = Exif::parse(&bytes).expect("round-trip parse");
        assert_eq!(parsed, original, "value-level round-trip in {order:?}");
        assert_eq!(parsed.byte_order(), order, "byte order preserved");
    }

    #[test]
    fn round_trips_both_byte_orders() {
        assert_round_trips(ByteOrder::LittleEndian);
        assert_round_trips(ByteOrder::BigEndian);
    }

    #[test]
    fn emits_and_omits_the_marker() {
        let exif = sample(ByteOrder::LittleEndian);
        let with = ExifWriter::new().write(&exif).expect("write");
        assert_eq!(&with[..6], MARKER);
        let bare = ExifWriter::new().marker(false).write(&exif).expect("write");
        assert_ne!(&bare[..2], MARKER);
        // A bare stream begins with the TIFF byte-order mark and re-parses.
        assert_eq!(&bare[..2], b"II");
        assert_eq!(Exif::parse(&bare).expect("bare re-parse"), exif);
    }

    #[test]
    fn byte_order_override_rewrites_endianness() {
        let exif = sample(ByteOrder::LittleEndian);
        let be = ExifWriter::new()
            .byte_order(ByteOrder::BigEndian)
            .write(&exif)
            .expect("write");
        let parsed = Exif::parse(&be).expect("parse");
        assert_eq!(parsed.byte_order(), ByteOrder::BigEndian);
        // Values are unchanged despite the re-encoding.
        assert_eq!(parsed.f_number(), exif.f_number());
        assert_eq!(
            parsed.get_tag(ExifTag::LensModel),
            exif.get_tag(ExifTag::LensModel)
        );
    }

    #[test]
    fn preserves_the_thumbnail_directory_on_round_trip() {
        // Build a blob with a 1st IFD (thumbnail) via gamut_ifd, parse it, then re-serialise: the
        // writer must chain the thumbnail directory back as the 1st IFD.
        let mut image = Ifd::new();
        image.set(ExifTag::Make.tag_id(), Value::Ascii("Canon".into()));
        let mut thumb = Ifd::new();
        thumb.set(ExifTag::Compression.tag_id(), Value::Short(vec![6]));
        thumb.set(
            ExifTag::JpegInterchangeFormatLength.tag_id(),
            Value::Long(vec![123]),
        );
        let blob = write(&TiffFile {
            order: ByteOrder::LittleEndian,
            variant: Variant::Classic,
            ifds: vec![image, thumb],
        })
        .expect("write");

        let parsed = Exif::parse(&blob).expect("parse");
        assert!(parsed.thumbnail_ifd().is_some());
        let reparsed = Exif::parse(&parsed.to_bytes().expect("write")).expect("re-parse");
        assert_eq!(
            reparsed, parsed,
            "thumbnail directory survives the round-trip"
        );
        assert_eq!(
            reparsed
                .thumbnail_ifd()
                .and_then(|t| t.get_u32(ExifTag::Compression.tag_id())),
            Some(6)
        );
    }

    #[test]
    fn re_embeds_a_jpeg_thumbnail_round_trip() {
        // Two JPEG lengths, one even and one odd, to exercise the word-alignment padding before the
        // appended bytes.
        for jpeg in [
            vec![0xFFu8, 0xD8, 0xFF, 0xD9],       // 4 bytes (even)
            vec![0xFFu8, 0xD8, 0xFF, 0xE0, 0xD9], // 5 bytes (odd)
        ] {
            let mut exif = sample(ByteOrder::LittleEndian);
            exif.set_thumbnail(jpeg.clone());

            let parsed = Exif::parse(&exif.to_bytes().expect("write")).expect("round-trip parse");
            assert_eq!(parsed.thumbnail_bytes(), Some(jpeg.as_slice()));
            assert_eq!(parsed.thumbnail().and_then(Thumbnail::compression), Some(6));
            // The rest of the model survives alongside the thumbnail.
            assert_eq!(parsed.make(), exif.make());
            assert_eq!(parsed.f_number(), exif.f_number());
        }
    }

    #[test]
    fn maker_note_bytes_survive_round_trip_verbatim() {
        // A MakerNote long enough to be stored out of line; its bytes must be byte-exact after a
        // round-trip even though v1 does not decode or rebase its internal offsets.
        let blob: Vec<u8> = (0..64u16).map(|b| b as u8).collect();
        let mut exif = Exif::new(ByteOrder::LittleEndian);
        exif.set_tag(ExifTag::Make, Value::Ascii("NIKON CORPORATION".into()));
        exif.set_tag(ExifTag::MakerNote, Value::Undefined(blob.clone()));

        let parsed = Exif::parse(&exif.to_bytes().expect("write")).expect("parse");
        let maker = parsed.maker_note().expect("maker note present");
        assert_eq!(maker.bytes, blob, "MakerNote bytes preserved verbatim");
        assert_eq!(maker.vendor, crate::MakerNoteVendor::Nikon);
    }

    #[test]
    fn hand_set_pointer_tags_do_not_corrupt_layout() {
        // A caller wrongly writes a raw ExifIFD pointer field; the writer must drop it and
        // synthesise the real one from the typed sub-IFD.
        let mut exif = Exif::new(ByteOrder::LittleEndian);
        exif.set_tag(ExifTag::Make, Value::Ascii("Canon".into()));
        exif.set_tag(ExifTag::FNumber, Value::Rational(vec![(28, 10)]));
        exif.image_mut()
            .set(EXIF_IFD_POINTER, Value::Long(vec![0xDEAD]));

        let parsed = Exif::parse(&exif.to_bytes().expect("write")).expect("parse");
        assert_eq!(
            parsed.f_number(),
            Some(crate::Rational { num: 28, den: 10 })
        );
        // The bogus pointer value is gone; the pointer is represented structurally.
        assert_eq!(parsed.get(IfdKind::Image, EXIF_IFD_POINTER), None);
    }
}
