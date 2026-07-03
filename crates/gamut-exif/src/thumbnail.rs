//! The embedded thumbnail — the 1st IFD and, for a JPEG thumbnail, its compressed bytes.
//!
//! Virtually all EXIF thumbnails are JPEG (`Compression` = 6, with `JPEGInterchangeFormat` /
//! `JPEGInterchangeFormatLength` pointing at the byte range). v1 extracts and re-embeds those
//! losslessly. An uncompressed strip-based thumbnail is preserved on read as its directory
//! ([`jpeg`](Thumbnail::jpeg) is `None`) but is **not** re-embedded on write — a documented v1
//! limitation, since its `StripOffsets` array would need the same offset-patching machinery.

use gamut_ifd::{Ifd, Value};

use crate::tag::ExifTag;

/// The image's embedded thumbnail: the 1st IFD's directory plus, for a JPEG thumbnail, its bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct Thumbnail {
    ifd: Ifd,
    jpeg: Option<Vec<u8>>,
}

impl Thumbnail {
    /// Assembles a thumbnail from an already-parsed 1st IFD and its extracted JPEG bytes (if any).
    pub(crate) fn from_parts(ifd: Ifd, jpeg: Option<Vec<u8>>) -> Self {
        Self { ifd, jpeg }
    }

    /// Builds a JPEG thumbnail from its compressed bytes, synthesising the 1st IFD tags
    /// (`Compression` = 6 and `JPEGInterchangeFormatLength`).
    ///
    /// The `JPEGInterchangeFormat` offset is *not* stored — it is structural (the bytes are held
    /// separately) and the writer synthesises it, so the model never carries a value that changes on
    /// every re-layout.
    #[must_use]
    pub fn from_jpeg(jpeg: Vec<u8>) -> Self {
        let mut ifd = Ifd::new();
        ifd.set(ExifTag::Compression.tag_id(), Value::Short(vec![6]));
        ifd.set(
            ExifTag::JpegInterchangeFormatLength.tag_id(),
            Value::Long(vec![jpeg.len() as u32]),
        );
        Self {
            ifd,
            jpeg: Some(jpeg),
        }
    }

    /// The thumbnail's directory (the 1st IFD).
    #[must_use]
    pub fn ifd(&self) -> &Ifd {
        &self.ifd
    }

    /// The thumbnail's directory, mutably (used when vivifying via [`Exif`](crate::Exif) accessors).
    pub(crate) fn ifd_mut(&mut self) -> &mut Ifd {
        &mut self.ifd
    }

    /// The compressed JPEG bytes, for a JPEG thumbnail; `None` for an uncompressed one.
    #[must_use]
    pub fn jpeg(&self) -> Option<&[u8]> {
        self.jpeg.as_deref()
    }

    /// The thumbnail's `Compression` tag (6 = JPEG, 1 = uncompressed), if present.
    #[must_use]
    pub fn compression(&self) -> Option<u16> {
        self.ifd
            .get_u32(ExifTag::Compression.tag_id())
            .and_then(|v| u16::try_from(v).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_jpeg_sets_the_1st_ifd_tags() {
        let thumb = Thumbnail::from_jpeg(vec![0xFF, 0xD8, 0xFF, 0xD9]);
        assert_eq!(thumb.jpeg(), Some(&[0xFF, 0xD8, 0xFF, 0xD9][..]));
        assert_eq!(thumb.compression(), Some(6));
        assert_eq!(
            thumb
                .ifd()
                .get_u32(ExifTag::JpegInterchangeFormatLength.tag_id()),
            Some(4)
        );
    }

    #[test]
    fn uncompressed_thumbnail_has_no_jpeg() {
        let mut ifd = Ifd::new();
        ifd.set(ExifTag::Compression.tag_id(), Value::Short(vec![1]));
        let thumb = Thumbnail::from_parts(ifd, None);
        assert_eq!(thumb.jpeg(), None);
        assert_eq!(thumb.compression(), Some(1));
    }
}
