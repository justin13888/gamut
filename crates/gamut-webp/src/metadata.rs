//! Embedded metadata: the `ICCP` colour profile and the `EXIF` / `XMP ` chunks of a WebP file
//! (RFC 9649 §2.7.2-§2.7.3).
//!
//! Payloads cross this boundary **verbatim**. The crate neither parses nor re-serializes them, so a
//! profile or packet read by [`metadata`] is byte-for-byte the one a writer embedded — the property
//! the typed metadata crates (`gamut-exif`, `gamut-icc`, `gamut-xmp`, and the `gamut-metadata`
//! facade) need in order to borrow the bytes without a copy or a re-frame. Encoding is the mirror
//! image: [`WebpEncoder::with_exif`](crate::WebpEncoder::with_exif),
//! [`with_xmp`](crate::WebpEncoder::with_xmp), and
//! [`with_icc_profile`](crate::WebpEncoder::with_icc_profile).

use gamut_core::Result;
use gamut_riff::MetadataChunks;

/// Embedded metadata read from a WebP file by [`metadata`].
///
/// Each payload is the raw chunk content, in the form the dedicated metadata crates parse (and
/// [`gamut-metadata`](https://crates.io/crates/gamut-metadata)'s `MetadataBlock` borrows) directly.
/// Marked `#[non_exhaustive]` so a further carrier can be added without a breaking change.
///
/// # Example
///
/// The extracted payload is byte-for-byte identical to the one given to the encoder, so it can be
/// borrowed straight into a typed metadata facade:
///
/// ```
/// use gamut_core::{Dimensions, EncodeImage, ImageRef, Rgb8};
/// use gamut_webp::WebpEncoder;
///
/// let icc = b"opaque ICC profile bytes";
/// let pixels = [10u8, 20, 30];
/// let image = ImageRef::<Rgb8>::new(&pixels, Dimensions::new(1, 1)?)?;
/// let mut file = Vec::new();
/// WebpEncoder::lossless()
///     .with_icc_profile(icc)
///     .encode_image(image, &mut file)?;
///
/// let meta = gamut_webp::metadata(&file)?;
/// assert_eq!(meta.icc.as_deref(), Some(icc.as_slice()));
/// # Ok::<(), gamut_core::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct WebpMetadata {
    /// The `EXIF` chunk payload: Exif metadata, carried bare (a WebP file stores no `"Exif\0\0"`
    /// signature, unlike a JPEG APP1 segment). Feed as `gamut_metadata::MetadataBlock::Exif`.
    pub exif: Option<Vec<u8>>,
    /// The `XMP ` chunk payload: an XMP packet. Feed as `MetadataBlock::Xmp`.
    pub xmp: Option<Vec<u8>>,
    /// The `ICCP` chunk payload: an ICC colour profile. `None` means sRGB is assumed (§2.7.2). Feed
    /// as `MetadataBlock::Icc`.
    pub icc: Option<Vec<u8>>,
}

/// Reads a WebP file's embedded metadata chunks without decoding any pixels.
///
/// Walks the top-level RIFF chunks and collects the three metadata payloads — `EXIF` and `XMP `
/// metadata (§2.7.3) and the `ICCP` colour profile (§2.7.2) — copying each out verbatim. The spec
/// permits at most one chunk of each kind and lets readers keep only the first, which is what this
/// does; the `VP8X` feature flags are advisory, so a payload is surfaced because its chunk is
/// present rather than because a flag advertises it. A simple (single-bitstream) file, which cannot
/// conformantly carry metadata at all, yields [`WebpMetadata::default`].
///
/// # Errors
///
/// Returns [`Error::InvalidInput`](gamut_core::Error::InvalidInput) if `data` is not a valid
/// RIFF/WebP file, or if a chunk's declared size runs past the end of the data.
///
/// # Example
///
/// ```
/// use gamut_core::{Dimensions, EncodeImage, ImageRef, Rgb8};
/// use gamut_webp::{WebpEncoder, WebpMetadata};
///
/// let pixels = [1u8, 2, 3];
/// let image = ImageRef::<Rgb8>::new(&pixels, Dimensions::new(1, 1)?)?;
/// let mut file = Vec::new();
/// WebpEncoder::lossless().encode_image(image, &mut file)?;
/// assert_eq!(gamut_webp::metadata(&file)?, WebpMetadata::default());
/// # Ok::<(), gamut_core::Error>(())
/// ```
pub fn metadata(data: &[u8]) -> Result<WebpMetadata> {
    let chunks = MetadataChunks::read(data)?;
    Ok(WebpMetadata {
        exif: chunks.exif.map(<[u8]>::to_vec),
        xmp: chunks.xmp.map(<[u8]>::to_vec),
        icc: chunks.icc.map(<[u8]>::to_vec),
    })
}
