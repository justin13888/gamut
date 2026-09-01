//! Reading an EXIF blob into the typed [`Exif`] model.
//!
//! An EXIF blob is an optional `Exif\0\0` marker followed by a TIFF stream. The 0th IFD and (when
//! present) the 1st IFD are the top-level chain [`gamut_ifd::read`] returns; the Exif, GPS, and
//! Interoperability directories hang off pointer *tags* that the generic reader cannot follow (it
//! cannot know which `LONG`s are offsets), so this reader chases those pointers explicitly and
//! removes them, representing each sub-IFD structurally on [`Exif`] instead.

use gamut_ifd::{ByteOrder, Ifd, IfdReader, Variant, tags as ifd_tags};

use crate::error::{ExifError, Result};
use crate::exif::{EXIF_IFD_POINTER, Exif, GPS_IFD_POINTER, INTEROP_IFD_POINTER, MARKER};
use crate::tag::ExifTag;
use crate::thumbnail::Thumbnail;

/// The absolute offset of the Exif sub-IFD's out-of-line `MakerNote` value in `tiff`, or `None`
/// if the note is absent or inline.
fn maker_note_offset(
    tiff: &[u8],
    exif_ifd_at: u64,
    order: ByteOrder,
    variant: Variant,
) -> Option<u64> {
    let mut reader = IfdReader::with_layout(tiff, order, variant);
    let raw = reader.read_ifd(exif_ifd_at).ok()?;
    let entry = raw.entry(ifd_tags::MAKER_NOTE)?;
    reader.value_offset(entry)
}

/// Reads an EXIF blob into an [`Exif`], with options for how the parse is bounded.
///
/// The default ([`ExifReader::new`]) accepts a blob with or without the `Exif\0\0` marker and is
/// lenient: a malformed Exif/GPS/Interop sub-IFD is dropped rather than failing the whole parse.
#[derive(Debug, Clone, Default)]
pub struct ExifReader {
    require_marker: bool,
    strict: bool,
}

impl ExifReader {
    /// A reader with default options (marker optional, lenient sub-IFD handling).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Requires the `Exif\0\0` marker; a bare TIFF stream is then rejected with
    /// [`ExifError::MissingMarker`].
    ///
    /// Off by default: the JPEG `APP1` segment carries the marker, but the WebP `EXIF` and PNG
    /// `eXIf` chunks carry a bare TIFF stream.
    #[must_use]
    pub fn require_marker(mut self, yes: bool) -> Self {
        self.require_marker = yes;
        self
    }

    /// In strict mode a malformed Exif/GPS/Interop sub-IFD fails the parse; by default it is
    /// dropped and the rest of the blob is returned.
    #[must_use]
    pub fn strict(mut self, yes: bool) -> Self {
        self.strict = yes;
        self
    }

    /// Parses an EXIF blob into an [`Exif`].
    ///
    /// # Errors
    ///
    /// Returns [`ExifError::MissingMarker`] when the marker is required but absent, an
    /// [`ExifError::Ifd`] when the TIFF stream is malformed, or (in [`strict`](Self::strict) mode)
    /// [`ExifError::InvalidIfd`] when a sub-IFD pointer addresses a malformed directory.
    pub fn parse(&self, bytes: &[u8]) -> Result<Exif> {
        let tiff = match bytes.strip_prefix(MARKER) {
            Some(rest) => rest,
            None if self.require_marker => return Err(ExifError::MissingMarker),
            None => bytes,
        };

        let file = gamut_ifd::read(tiff)?;
        let order = file.order;
        let variant = file.variant;
        let mut ifds = file.ifds.into_iter();
        let mut image = ifds.next().ok_or(ExifError::Truncated)?;
        // The next-IFD chain's second entry is the thumbnail directory (1st IFD), if any.
        let thumbnail = match ifds.next() {
            Some(ifd) => Some(self.read_thumbnail(ifd, tiff)?),
            None => None,
        };

        // The Exif sub-IFD's own offset, captured before `follow` strips the pointer: the
        // maker-note pin needs the note value's absolute source position.
        let exif_ifd_at = image.get_u32(EXIF_IFD_POINTER).map(u64::from);
        let exif = self.follow(&mut image, tiff, order, variant, EXIF_IFD_POINTER, "Exif")?;
        let gps = self.follow(&mut image, tiff, order, variant, GPS_IFD_POINTER, "GPS")?;
        let maker_note_at = match (&exif, exif_ifd_at) {
            (Some(_), Some(at)) => maker_note_offset(tiff, at, order, variant),
            _ => None,
        };

        // The Interoperability directory is reached from *inside* the Exif sub-IFD, not the 0th IFD.
        let (exif, interop) = match exif {
            Some(mut e) => {
                let interop =
                    self.follow(&mut e, tiff, order, variant, INTEROP_IFD_POINTER, "Interop")?;
                (Some(e), interop)
            }
            None => (None, None),
        };

        Ok(Exif::from_parts(
            order,
            image,
            exif,
            gps,
            interop,
            thumbnail,
            maker_note_at,
        ))
    }

    /// Reads pointer tag `ptr` from `parent`, removes it (the pointer is represented structurally,
    /// not as a data field), and parses the sub-IFD it addresses.
    ///
    /// Returns `Ok(None)` when the pointer is absent, or — in lenient mode — when the pointed-at
    /// directory is malformed.
    fn follow(
        &self,
        parent: &mut Ifd,
        tiff: &[u8],
        order: ByteOrder,
        variant: Variant,
        ptr: u16,
        name: &'static str,
    ) -> Result<Option<Ifd>> {
        let Some(offset) = parent.get_u32(ptr) else {
            return Ok(None);
        };
        parent.remove(ptr);
        match gamut_ifd::read_ifd_at(tiff, u64::from(offset), order, variant) {
            Ok(ifd) => Ok(Some(ifd)),
            Err(_) if !self.strict => Ok(None),
            Err(_) => Err(ExifError::InvalidIfd(name)),
        }
    }

    /// Builds a [`Thumbnail`] from the 1st IFD, slicing out its JPEG bytes (from the
    /// `JPEGInterchangeFormat` offset / length) when present. In lenient mode an out-of-bounds
    /// JPEG range yields a thumbnail without bytes; in strict mode it errors.
    fn read_thumbnail(&self, ifd: Ifd, tiff: &[u8]) -> Result<Thumbnail> {
        let offset = ifd.get_u32(ExifTag::JpegInterchangeFormat.tag_id());
        let length = ifd.get_u32(ExifTag::JpegInterchangeFormatLength.tag_id());
        let jpeg = match (offset, length) {
            (Some(offset), Some(length)) => {
                let range = (offset as usize).checked_add(length as usize);
                match range.and_then(|end| tiff.get(offset as usize..end)) {
                    Some(bytes) => Some(bytes.to_vec()),
                    None if self.strict => {
                        return Err(ExifError::BadThumbnail("JPEG offset out of bounds"));
                    }
                    None => None,
                }
            }
            _ => None,
        };
        // The JPEGInterchangeFormat offset is structural — the bytes are captured above and the
        // writer re-synthesises the offset — so drop it from the stored directory (mirroring how the
        // sub-IFD pointer tags are stripped), leaving a value the model can't carry stale.
        let mut ifd = ifd;
        if jpeg.is_some() {
            ifd.remove(ExifTag::JpegInterchangeFormat.tag_id());
        }
        Ok(Thumbnail::from_parts(ifd, jpeg))
    }
}

#[cfg(test)]
mod tests {
    use gamut_ifd::{TiffFile, Value, write};

    use super::*;
    use crate::IfdKind;

    /// Builds a small but structurally complete EXIF TIFF stream (0th IFD with Make/Orientation,
    /// an Exif sub-IFD with FNumber + a nested Interop sub-IFD, a GPS sub-IFD, and a thumbnail
    /// 1st IFD), optionally prefixed with the `Exif\0\0` marker.
    fn sample_blob(order: ByteOrder, with_marker: bool) -> Vec<u8> {
        let mut image = Ifd::new();
        image.set(0x010F, Value::Ascii("Canon".into())); // Make
        image.set(0x0112, Value::Short(vec![1])); // Orientation

        let mut interop = Ifd::new();
        interop.set(0x0001, Value::Ascii("R98".into())); // InteroperabilityIndex

        let mut exif = Ifd::new();
        exif.set(0x829D, Value::Rational(vec![(28, 10)])); // FNumber
        exif.set(0x8827, Value::Short(vec![400])); // PhotographicSensitivity (ISO)
        exif.set_sub_ifd(INTEROP_IFD_POINTER, vec![interop]);

        let mut gps = Ifd::new();
        gps.set(0x0000, Value::Byte(vec![2, 3, 0, 0])); // GPSVersionID

        image.set_sub_ifd(EXIF_IFD_POINTER, vec![exif]);
        image.set_sub_ifd(GPS_IFD_POINTER, vec![gps]);

        let mut thumb = Ifd::new();
        thumb.set(0x0103, Value::Short(vec![6])); // Compression = JPEG

        let bytes = write(&TiffFile {
            order,
            variant: Variant::Classic,
            ifds: vec![image, thumb],
        })
        .expect("write");
        if with_marker {
            let mut out = MARKER.to_vec();
            out.extend(bytes);
            out
        } else {
            bytes
        }
    }

    fn assert_parsed(exif: &Exif, order: ByteOrder) {
        assert_eq!(exif.byte_order(), order);
        assert_eq!(exif.make(), Some("Canon"));
        assert_eq!(exif.orientation(), Some(1));
        // Sub-IFDs were followed and typed accessors reach into them.
        assert_eq!(exif.f_number(), Some(crate::Rational { num: 28, den: 10 }));
        assert_eq!(exif.iso(), Some(400));
        assert!(exif.gps_ifd().is_some());
        assert_eq!(
            exif.get(IfdKind::Interop, 0x0001),
            Some(&Value::Ascii("R98".into()))
        );
        assert!(exif.thumbnail_ifd().is_some());
        // The pointer tags were stripped — they are represented structurally, not as data.
        assert_eq!(exif.get(IfdKind::Image, EXIF_IFD_POINTER), None);
        assert_eq!(exif.get(IfdKind::Image, GPS_IFD_POINTER), None);
        assert_eq!(exif.get(IfdKind::Exif, INTEROP_IFD_POINTER), None);
    }

    #[test]
    fn parses_both_byte_orders_with_marker() {
        for order in [ByteOrder::LittleEndian, ByteOrder::BigEndian] {
            let exif = Exif::parse(&sample_blob(order, true)).expect("parse");
            assert_parsed(&exif, order);
        }
    }

    #[test]
    fn parses_bare_tiff_without_marker() {
        let exif = Exif::parse(&sample_blob(ByteOrder::LittleEndian, false)).expect("parse");
        assert_parsed(&exif, ByteOrder::LittleEndian);
    }

    #[test]
    fn require_marker_rejects_bare_tiff() {
        let bare = sample_blob(ByteOrder::LittleEndian, false);
        let err = ExifReader::new()
            .require_marker(true)
            .parse(&bare)
            .expect_err("bare TIFF must be rejected");
        assert!(matches!(err, ExifError::MissingMarker));
        // ...but the same reader still accepts the marked form.
        let marked = sample_blob(ByteOrder::LittleEndian, true);
        assert!(
            ExifReader::new()
                .require_marker(true)
                .parse(&marked)
                .is_ok()
        );
    }

    #[test]
    fn malformed_stream_errors() {
        assert!(Exif::parse(b"not tiff at all").is_err());
        assert!(Exif::parse(&[]).is_err());
    }

    /// The thumbnail's JPEG range is bounds-checked, and the two modes disagree about it.
    ///
    /// `read_thumbnail` documents that "in lenient mode an out-of-bounds JPEG range yields a
    /// thumbnail without bytes; in strict mode it errors" -- and **neither branch had a test**
    /// (#110). Both mutation directions of that `if self.strict` guard survived, which is what
    /// "documented behaviour, zero coverage" looks like from the outside.
    ///
    /// This is a decode path fed untrusted input, so the bound is the point: without it the slice
    /// would be taken from a hostile offset.
    #[test]
    fn an_out_of_bounds_thumbnail_jpeg_is_dropped_leniently_and_rejected_strictly() {
        let mut image = Ifd::new();
        image.set(0x010F, Value::Ascii("Canon".into()));
        let mut thumb = Ifd::new();
        // A JPEG that claims to start far past the end of the stream.
        thumb.set(
            ExifTag::JpegInterchangeFormat.tag_id(),
            Value::Long(vec![0xFFFF]),
        );
        thumb.set(
            ExifTag::JpegInterchangeFormatLength.tag_id(),
            Value::Long(vec![16]),
        );
        let bytes = write(&TiffFile {
            order: ByteOrder::LittleEndian,
            variant: Variant::Classic,
            ifds: vec![image, thumb],
        })
        .expect("write");

        // Lenient: the thumbnail survives, minus the bytes it could not have.
        let lenient = ExifReader::new().parse(&bytes).expect("lenient parse");
        let t = lenient
            .thumbnail()
            .expect("the 1st IFD is still a thumbnail");
        assert_eq!(t.jpeg(), None, "no bytes, rather than bytes from nowhere");
        assert_eq!(
            lenient.make(),
            Some("Canon"),
            "the rest of the file survives"
        );

        // Strict: the same input is refused.
        let err = ExifReader::new()
            .strict(true)
            .parse(&bytes)
            .expect_err("strict must reject an out-of-bounds thumbnail");
        assert!(
            matches!(err, ExifError::BadThumbnail(_)),
            "wrong error for an out-of-bounds thumbnail: {err:?}"
        );
    }

    #[test]
    fn lenient_drops_a_dangling_sub_ifd_pointer_that_strict_rejects() {
        // An ExifIFD pointer that addresses far past the end of the stream.
        let mut image = Ifd::new();
        image.set(0x010F, Value::Ascii("Canon".into()));
        image.set(EXIF_IFD_POINTER, Value::Long(vec![0xFFFF]));
        let bytes = write(&TiffFile {
            order: ByteOrder::LittleEndian,
            variant: Variant::Classic,
            ifds: vec![image],
        })
        .expect("write");

        // Lenient: the bad pointer is dropped, the rest survives.
        let lenient = ExifReader::new().parse(&bytes).expect("lenient parse");
        assert_eq!(lenient.make(), Some("Canon"));
        assert!(lenient.exif_ifd().is_none());
        assert_eq!(lenient.get(IfdKind::Image, EXIF_IFD_POINTER), None);

        // Strict: the malformed sub-IFD fails the parse.
        let err = ExifReader::new()
            .strict(true)
            .parse(&bytes)
            .expect_err("strict must reject");
        assert!(matches!(err, ExifError::InvalidIfd("Exif")));
    }
}
