//! The parsed EXIF model: the IFD chain that makes up an EXIF blob.

use gamut_ifd::{ByteOrder, Ifd, Value};

use crate::gps::GpsInfo;
use crate::tag::{ExifTag, IfdKind};
use crate::thumbnail::Thumbnail;
use crate::value::{Rational, as_text};

/// The 6-byte identifier that precedes the TIFF stream in a JPEG `APP1` EXIF segment.
pub(crate) const MARKER: &[u8] = b"Exif\x00\x00";
/// `ExifIFD` pointer (0th IFD → Exif sub-IFD), Exif 3.0 §4.6.3.
pub(crate) const EXIF_IFD_POINTER: u16 = 0x8769;
/// `GPSInfo` pointer (0th IFD → GPS sub-IFD).
pub(crate) const GPS_IFD_POINTER: u16 = 0x8825;
/// `Interoperability` pointer (Exif sub-IFD → Interop sub-IFD).
pub(crate) const INTEROP_IFD_POINTER: u16 = 0xA005;

/// A parsed EXIF blob — the directories of the TIFF stream that follows the optional `Exif\0\0`
/// marker.
///
/// The 0th IFD (`image`) holds the primary-image tags and the pointer tags that reach the
/// Exif/GPS/Interop sub-IFDs; the 1st IFD holds the thumbnail. The byte order is preserved so a
/// re-serialised blob can match the source's endianness.
///
/// Read a blob with [`Exif::parse`] and re-emit it with [`Exif::to_bytes`]; build one from scratch
/// with [`Exif::new`]. Fields are private so the representation can evolve without breaking the 1.0
/// API — reach the directories through the accessors, which hand back the underlying
/// [`gamut_ifd::Ifd`] that consumers already speak.
///
/// The pointer tags — `ExifIFD` (`0x8769`), `GPSInfo` (`0x8825`), and `Interoperability`
/// (`0xA005`) — are **managed by the crate**: the writer synthesises them from the typed sub-IFDs,
/// so set the sub-IFDs (e.g. [`set_exif_ifd`](Self::set_exif_ifd)), never the pointer fields.
#[derive(Debug, Clone, PartialEq)]
pub struct Exif {
    order: ByteOrder,
    image: Ifd,
    exif: Option<Ifd>,
    gps: Option<Ifd>,
    interop: Option<Ifd>,
    thumbnail: Option<Thumbnail>,
}

impl Exif {
    /// Creates an EXIF blob with an empty 0th IFD and the given byte order.
    #[must_use]
    pub fn new(order: ByteOrder) -> Self {
        Self {
            order,
            image: Ifd::new(),
            exif: None,
            gps: None,
            interop: None,
            thumbnail: None,
        }
    }

    /// Assembles an [`Exif`] from already-parsed directories (used by the reader).
    pub(crate) fn from_parts(
        order: ByteOrder,
        image: Ifd,
        exif: Option<Ifd>,
        gps: Option<Ifd>,
        interop: Option<Ifd>,
        thumbnail: Option<Thumbnail>,
    ) -> Self {
        Self {
            order,
            image,
            exif,
            gps,
            interop,
            thumbnail,
        }
    }

    /// Parses an EXIF blob (with or without the `Exif\0\0` marker) with default options.
    ///
    /// Equivalent to [`ExifReader::new().parse(bytes)`](crate::ExifReader::parse). For control over
    /// marker handling or strictness, use [`ExifReader`](crate::ExifReader) directly.
    ///
    /// # Errors
    ///
    /// Returns an [`ExifError`](crate::ExifError) if the TIFF stream is malformed.
    pub fn parse(bytes: &[u8]) -> crate::Result<Self> {
        crate::ExifReader::new().parse(bytes)
    }

    /// Serialises this EXIF blob to a valid `Exif\0\0` + TIFF stream with default options.
    ///
    /// Preserves the source byte order and re-synthesises the Exif/GPS/Interop pointer tags. For a
    /// bare TIFF stream (PNG `eXIf` / WebP `EXIF`) or a byte-order override, use
    /// [`ExifWriter`](crate::ExifWriter).
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        crate::ExifWriter::new().write(self)
    }

    /// The byte order of the underlying TIFF stream (preserved for round-tripping).
    #[must_use]
    pub fn byte_order(&self) -> ByteOrder {
        self.order
    }

    /// The 0th IFD — primary-image / TIFF tags.
    #[must_use]
    pub fn image(&self) -> &Ifd {
        &self.image
    }

    /// The 0th IFD, mutably.
    pub fn image_mut(&mut self) -> &mut Ifd {
        &mut self.image
    }

    /// The Exif sub-IFD (capture parameters), if present.
    #[must_use]
    pub fn exif_ifd(&self) -> Option<&Ifd> {
        self.exif.as_ref()
    }

    /// The Exif sub-IFD, mutably, creating an empty one if absent.
    pub fn exif_ifd_mut(&mut self) -> &mut Ifd {
        self.exif.get_or_insert_with(Ifd::new)
    }

    /// The GPS sub-IFD, if present.
    #[must_use]
    pub fn gps_ifd(&self) -> Option<&Ifd> {
        self.gps.as_ref()
    }

    /// The GPS sub-IFD, mutably, creating an empty one if absent.
    pub fn gps_ifd_mut(&mut self) -> &mut Ifd {
        self.gps.get_or_insert_with(Ifd::new)
    }

    /// The Interoperability sub-IFD, if present.
    #[must_use]
    pub fn interop_ifd(&self) -> Option<&Ifd> {
        self.interop.as_ref()
    }

    /// The Interoperability sub-IFD, mutably, creating an empty one if absent.
    pub fn interop_ifd_mut(&mut self) -> &mut Ifd {
        self.interop.get_or_insert_with(Ifd::new)
    }

    /// The embedded thumbnail (1st IFD), if present.
    #[must_use]
    pub fn thumbnail(&self) -> Option<&Thumbnail> {
        self.thumbnail.as_ref()
    }

    /// The 1st IFD — the embedded thumbnail's directory, if present.
    #[must_use]
    pub fn thumbnail_ifd(&self) -> Option<&Ifd> {
        self.thumbnail.as_ref().map(Thumbnail::ifd)
    }

    /// The embedded JPEG thumbnail's compressed bytes, if there is a JPEG thumbnail.
    #[must_use]
    pub fn thumbnail_bytes(&self) -> Option<&[u8]> {
        self.thumbnail.as_ref().and_then(Thumbnail::jpeg)
    }

    /// Sets (or replaces) the embedded thumbnail to a JPEG image.
    pub fn set_thumbnail(&mut self, jpeg: Vec<u8>) {
        self.thumbnail = Some(Thumbnail::from_jpeg(jpeg));
    }

    /// Removes the embedded thumbnail, if any.
    pub fn clear_thumbnail(&mut self) {
        self.thumbnail = None;
    }

    /// Replaces the Exif sub-IFD.
    pub fn set_exif_ifd(&mut self, ifd: Ifd) {
        self.exif = Some(ifd);
    }

    /// Replaces the GPS sub-IFD.
    pub fn set_gps_ifd(&mut self, ifd: Ifd) {
        self.gps = Some(ifd);
    }

    /// Replaces the Interoperability sub-IFD.
    pub fn set_interop_ifd(&mut self, ifd: Ifd) {
        self.interop = Some(ifd);
    }

    /// Returns the value of `tag` in directory `ifd`, or `None` if that directory or tag is absent.
    #[must_use]
    pub fn get(&self, ifd: IfdKind, tag: u16) -> Option<&Value> {
        self.directory(ifd).and_then(|d| d.get(tag))
    }

    /// Sets `tag` to `value` in directory `ifd`, creating the sub-IFD if it does not yet exist.
    pub fn set(&mut self, ifd: IfdKind, tag: u16, value: Value) {
        self.directory_mut(ifd).set(tag, value);
    }

    /// Returns the value of a catalogued [`ExifTag`], looked up in its home directory.
    #[must_use]
    pub fn get_tag(&self, tag: ExifTag) -> Option<&Value> {
        self.get(tag.ifd(), tag.tag_id())
    }

    /// Sets a catalogued [`ExifTag`] to `value` in its home directory, creating the sub-IFD if
    /// needed.
    pub fn set_tag(&mut self, tag: ExifTag, value: Value) {
        self.set(tag.ifd(), tag.tag_id(), value);
    }

    /// The camera manufacturer (`Make`, 0th IFD).
    #[must_use]
    pub fn make(&self) -> Option<&str> {
        self.get_tag(ExifTag::Make).and_then(as_text)
    }

    /// The camera model (`Model`, 0th IFD).
    #[must_use]
    pub fn model(&self) -> Option<&str> {
        self.get_tag(ExifTag::Model).and_then(as_text)
    }

    /// The creating software (`Software`, 0th IFD).
    #[must_use]
    pub fn software(&self) -> Option<&str> {
        self.get_tag(ExifTag::Software).and_then(as_text)
    }

    /// The image orientation (`Orientation`, 0th IFD): 1–8 per the TIFF/Exif convention.
    #[must_use]
    pub fn orientation(&self) -> Option<u16> {
        self.get_tag(ExifTag::Orientation)
            .and_then(Value::as_u32)
            .and_then(|v| u16::try_from(v).ok())
    }

    /// The capture time (`DateTimeOriginal`, Exif IFD), as its raw `YYYY:MM:DD HH:MM:SS` string.
    #[must_use]
    pub fn datetime_original(&self) -> Option<&str> {
        self.get_tag(ExifTag::DateTimeOriginal).and_then(as_text)
    }

    /// The exposure time in seconds (`ExposureTime`, Exif IFD).
    #[must_use]
    pub fn exposure_time(&self) -> Option<Rational> {
        self.get_tag(ExifTag::ExposureTime).and_then(first_rational)
    }

    /// The lens f-number (`FNumber`, Exif IFD).
    #[must_use]
    pub fn f_number(&self) -> Option<Rational> {
        self.get_tag(ExifTag::FNumber).and_then(first_rational)
    }

    /// The ISO sensitivity (`PhotographicSensitivity`, Exif IFD).
    #[must_use]
    pub fn iso(&self) -> Option<u32> {
        self.get_tag(ExifTag::PhotographicSensitivity)
            .and_then(Value::as_u32)
    }

    /// The focal length in millimetres (`FocalLength`, Exif IFD).
    #[must_use]
    pub fn focal_length(&self) -> Option<Rational> {
        self.get_tag(ExifTag::FocalLength).and_then(first_rational)
    }

    /// The lens model (`LensModel`, Exif IFD).
    #[must_use]
    pub fn lens_model(&self) -> Option<&str> {
        self.get_tag(ExifTag::LensModel).and_then(as_text)
    }

    /// The GPS position as a typed [`GpsInfo`], or `None` if there is no GPS sub-IFD or it holds no
    /// positioning tags. The full GPS directory is always available via [`gps_ifd`](Self::gps_ifd).
    #[must_use]
    pub fn gps(&self) -> Option<GpsInfo> {
        GpsInfo::from_ifd(self.gps_ifd()?)
    }

    /// The directory for `kind`, if present.
    fn directory(&self, kind: IfdKind) -> Option<&Ifd> {
        match kind {
            IfdKind::Image => Some(&self.image),
            IfdKind::Exif => self.exif.as_ref(),
            IfdKind::Gps => self.gps.as_ref(),
            IfdKind::Interop => self.interop.as_ref(),
            IfdKind::Thumbnail => self.thumbnail.as_ref().map(Thumbnail::ifd),
        }
    }

    /// The directory for `kind`, creating (vivifying) it if absent.
    fn directory_mut(&mut self, kind: IfdKind) -> &mut Ifd {
        match kind {
            IfdKind::Image => &mut self.image,
            IfdKind::Exif => self.exif.get_or_insert_with(Ifd::new),
            IfdKind::Gps => self.gps.get_or_insert_with(Ifd::new),
            IfdKind::Interop => self.interop.get_or_insert_with(Ifd::new),
            IfdKind::Thumbnail => self
                .thumbnail
                .get_or_insert_with(|| Thumbnail::from_parts(Ifd::new(), None))
                .ifd_mut(),
        }
    }
}

/// Extracts the first element of a `RATIONAL` value as a [`Rational`], or `None` for other types.
fn first_rational(value: &Value) -> Option<Rational> {
    match value {
        Value::Rational(rs) => rs.first().copied().map(Rational::from),
        _ => None,
    }
}

/// Returns a copy of `ifd` with every field in `tags` removed.
///
/// [`gamut_ifd::Ifd`] exposes no in-place removal; the reader uses this to drop pointer tags after
/// following them, and the writer to drop any hand-set pointer tags before re-synthesising them.
pub(crate) fn without_tags(ifd: &Ifd, tags: &[u16]) -> Ifd {
    let mut out = Ifd::new();
    for field in ifd.fields() {
        if !tags.contains(&field.tag) {
            out.set(field.tag, field.value.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_starts_empty_with_the_given_order() {
        let exif = Exif::new(ByteOrder::BigEndian);
        assert_eq!(exif.byte_order(), ByteOrder::BigEndian);
        assert!(exif.image().fields().is_empty());
        assert!(exif.exif_ifd().is_none());
        assert!(exif.gps_ifd().is_none());
        assert!(exif.interop_ifd().is_none());
        assert!(exif.thumbnail_ifd().is_none());
    }

    #[test]
    fn set_vivifies_sub_ifds_and_get_reads_back() {
        let mut exif = Exif::new(ByteOrder::LittleEndian);
        exif.set(IfdKind::Image, 0x010F, Value::Ascii("NIKON".into()));
        exif.set(IfdKind::Exif, 0x8827, Value::Short(vec![400]));
        exif.set(IfdKind::Gps, 0x0000, Value::Byte(vec![2, 3, 0, 0]));

        assert_eq!(
            exif.get(IfdKind::Image, 0x010F),
            Some(&Value::Ascii("NIKON".into()))
        );
        assert_eq!(
            exif.get(IfdKind::Exif, 0x8827),
            Some(&Value::Short(vec![400]))
        );
        // The Exif and GPS sub-IFDs were created on first write.
        assert!(exif.exif_ifd().is_some());
        assert!(exif.gps_ifd().is_some());
        // Absent directory / tag both read back as None.
        assert_eq!(exif.get(IfdKind::Interop, 0x0001), None);
        assert_eq!(exif.get(IfdKind::Image, 0x0100), None);
    }

    #[test]
    fn mut_accessors_vivify_and_set_replaces() {
        let mut exif = Exif::new(ByteOrder::LittleEndian);
        exif.exif_ifd_mut()
            .set(0x9000, Value::Undefined(b"0300".to_vec()));
        assert_eq!(
            exif.get(IfdKind::Exif, 0x9000),
            Some(&Value::Undefined(b"0300".to_vec()))
        );

        let mut replacement = Ifd::new();
        replacement.set(0x0001, Value::Byte(vec![b'N']));
        exif.set_gps_ifd(replacement);
        assert_eq!(
            exif.get(IfdKind::Gps, 0x0001),
            Some(&Value::Byte(vec![b'N']))
        );
    }

    #[test]
    fn set_tag_and_get_tag_route_by_home_ifd() {
        let mut exif = Exif::new(ByteOrder::LittleEndian);
        exif.set_tag(ExifTag::Make, Value::Ascii("Canon".into()));
        exif.set_tag(ExifTag::FNumber, Value::Rational(vec![(28, 10)]));

        // Make lives in the 0th IFD; FNumber in the Exif sub-IFD.
        assert_eq!(
            exif.get_tag(ExifTag::Make),
            Some(&Value::Ascii("Canon".into()))
        );
        assert_eq!(
            exif.get(IfdKind::Image, 0x010F),
            Some(&Value::Ascii("Canon".into()))
        );
        assert_eq!(
            exif.get_tag(ExifTag::FNumber),
            Some(&Value::Rational(vec![(28, 10)]))
        );
        assert!(exif.exif_ifd().is_some());
        assert_eq!(exif.get_tag(ExifTag::Model), None);
    }
}
