//! Writing an [`Exif`] back to a valid EXIF blob.
//!
//! The writer hands the whole IFD tree to [`gamut_ifd::write`], whose two-pass offset layout does
//! the hard part: it places every directory, appends one value pool, and synthesises each sub-IFD
//! pointer field with the child's patched offset. This crate's job is only to shape the tree —
//! attach the Exif/GPS/Interop sub-IFDs under their pointer tags and chain the thumbnail as the 1st
//! IFD — so the round-trip `parse → write → parse` reproduces the directories with the source byte
//! order preserved.

use gamut_ifd::{ByteOrder, Ifd, TiffFile, Variant, write};

use crate::exif::{
    EXIF_IFD_POINTER, Exif, GPS_IFD_POINTER, INTEROP_IFD_POINTER, MARKER, without_tags,
};

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
    #[must_use]
    pub fn write(&self, exif: &Exif) -> Vec<u8> {
        let order = self.byte_order.unwrap_or_else(|| exif.byte_order());

        // Build the Exif sub-IFD, nesting the Interoperability sub-IFD under its pointer tag. An
        // Interop directory implies an Exif directory to hold its pointer, so vivify one if needed.
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

        // Rebuild the 0th IFD, dropping any hand-set pointer tags and re-attaching the sub-IFDs so
        // gamut_ifd::write synthesises correct pointer offsets.
        let mut image = without_tags(exif.image(), &[EXIF_IFD_POINTER, GPS_IFD_POINTER]);
        if let Some(e) = exif_sub {
            image.set_sub_ifd(EXIF_IFD_POINTER, vec![e]);
        }
        if let Some(gps) = exif.gps_ifd() {
            image.set_sub_ifd(GPS_IFD_POINTER, vec![gps.clone()]);
        }

        let mut ifds = vec![image];
        if let Some(thumb) = exif.thumbnail_ifd() {
            ifds.push(thumb.clone());
        }

        let bytes = write(&TiffFile {
            order,
            variant: Variant::Classic,
            ifds,
        });

        if self.marker {
            let mut out = MARKER.to_vec();
            out.extend(bytes);
            out
        } else {
            bytes
        }
    }
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
        let bytes = original.to_bytes();
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
        let with = ExifWriter::new().write(&exif);
        assert_eq!(&with[..6], MARKER);
        let bare = ExifWriter::new().marker(false).write(&exif);
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
            .write(&exif);
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
        });

        let parsed = Exif::parse(&blob).expect("parse");
        assert!(parsed.thumbnail_ifd().is_some());
        let reparsed = Exif::parse(&parsed.to_bytes()).expect("re-parse");
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
    fn hand_set_pointer_tags_do_not_corrupt_layout() {
        // A caller wrongly writes a raw ExifIFD pointer field; the writer must drop it and
        // synthesise the real one from the typed sub-IFD.
        let mut exif = Exif::new(ByteOrder::LittleEndian);
        exif.set_tag(ExifTag::Make, Value::Ascii("Canon".into()));
        exif.set_tag(ExifTag::FNumber, Value::Rational(vec![(28, 10)]));
        exif.image_mut()
            .set(EXIF_IFD_POINTER, Value::Long(vec![0xDEAD]));

        let parsed = Exif::parse(&exif.to_bytes()).expect("parse");
        assert_eq!(
            parsed.f_number(),
            Some(crate::Rational { num: 28, den: 10 })
        );
        // The bogus pointer value is gone; the pointer is represented structurally.
        assert_eq!(parsed.get(IfdKind::Image, EXIF_IFD_POINTER), None);
    }
}
