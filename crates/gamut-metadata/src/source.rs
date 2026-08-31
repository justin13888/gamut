//! Carrier-agnostic metadata blocks.

/// A located metadata payload, handed over by a container crate for parsing.
///
/// Each variant borrows the raw bytes a container has already extracted — a WebP `EXIF`/`XMP `/
/// `ICCP` chunk, an AVIF/HEIF `Exif`/`mime`/`colr` item payload, a JPEG `APP1`/`APP2`/`APP13`
/// segment. The facade stays container-agnostic: it never parses boxes or chunks, only these
/// payloads. (IPTC Core/Extension arrives inside an [`MetadataBlock::Xmp`] payload; the separate
/// [`MetadataBlock::IptcIim`] is the legacy binary form from a Photoshop IRB.)
///
/// Marked `#[non_exhaustive]`, so a later carrier can add a variant without a breaking change;
/// match with a wildcard arm. There is deliberately **no** variant for
/// [`Metadata::extensions`](crate::Metadata::extensions): extensions hold data no carrier
/// serializes, so no block can produce one. A C2PA manifest store is the opposite case — a
/// container does hold it — so it has a variant, [`MetadataBlock::C2pa`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MetadataBlock<'a> {
    /// An EXIF blob (`Exif\0\0` + TIFF stream, or a bare TIFF stream).
    Exif(&'a [u8]),
    /// An XMP packet (RDF/XML).
    Xmp(&'a [u8]),
    /// An ICC profile blob.
    Icc(&'a [u8]),
    /// A legacy IPTC-IIM dataset stream (e.g. the `0x0404` Photoshop image resource's payload).
    IptcIim(&'a [u8]),
    /// A C2PA manifest store: the JUMBF superbox a container located (C2PA 2.4 §11.1.1), taken
    /// verbatim.
    ///
    /// The facade never looks inside it. Extraction hands the bytes to
    /// [`Metadata::c2pa`](crate::Metadata::c2pa); embedding does **not** hand them back — see
    /// [`C2paPolicy`](crate::C2paPolicy) for why a store must not be copied into a rewritten file.
    ///
    /// **Hand over the complete store.** Where a carrier splits it across segments — C2PA's JPEG
    /// carriage spans as many `APP11` segments as the store needs, which is the ordinary case once
    /// a manifest embeds a thumbnail — the container reassembles them first, exactly as it already
    /// does for a multi-segment `APP2 ICC_PROFILE`. One block per segment would not error: a
    /// repeated block kind takes the last occurrence, so the model would quietly hold a fragment.
    C2pa(&'a [u8]),
}
