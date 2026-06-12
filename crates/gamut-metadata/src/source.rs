//! Carrier-agnostic metadata blocks.

/// A located metadata payload, handed over by a container crate for parsing.
///
/// Each variant borrows the raw bytes a container has already extracted — a WebP `EXIF`/`XMP `/
/// `ICCP` chunk, an AVIF/HEIF `Exif`/`mime`/`colr` item payload, a JPEG `APP1`/`APP2`/`APP13`
/// segment. The facade stays container-agnostic: it never parses boxes or chunks, only these
/// payloads. (IPTC Core/Extension arrives inside an [`MetadataBlock::Xmp`] payload; the separate
/// [`MetadataBlock::IptcIim`] is the legacy binary form from a Photoshop IRB.)
pub enum MetadataBlock<'a> {
    /// An EXIF blob (`Exif\0\0` + TIFF stream).
    Exif(&'a [u8]),
    /// An XMP packet (RDF/XML).
    Xmp(&'a [u8]),
    /// An ICC profile blob.
    Icc(&'a [u8]),
    /// A legacy IPTC-IIM dataset stream (e.g. the `0x0404` Photoshop image resource).
    IptcIim(&'a [u8]),
}
