//! WebP-specific helpers over the generic RIFF layer: classifying WebP chunks, the [`Vp8xHeader`]
//! extended-format feature header, the [`MetadataChunks`] passthrough for `ICCP`/`EXIF`/`XMP `, and
//! writing the simple (single-bitstream) and extended file formats (RFC 9649 §2.5-§2.7).
//!
//! The remaining extended-format chunks (`ANIM`/`ANMF`) are tracked in `gamut-webp/STATUS.md`
//! section A and are out of scope under the image-first charter.

use gamut_core::{Error, Result};

use crate::fourcc::FourCc;
use crate::reader::RiffReader;
use crate::writer::RiffWriter;

/// The number of bytes in a `VP8X` chunk payload (RFC 9649 §2.7).
pub const VP8X_PAYLOAD_LEN: usize = 10;

/// The extended-format feature header carried by a `VP8X` chunk (RFC 9649 §2.7): which optional
/// features the file uses, plus the 1-based canvas dimensions. A simple (single-bitstream) file has no
/// `VP8X` chunk; one is required as soon as the file carries alpha, an ICC profile, metadata, or
/// animation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Vp8xHeader {
    /// The file contains an `ICCP` (ICC color profile) chunk.
    pub icc_profile: bool,
    /// The image carries transparency (an `ALPH` chunk, or alpha in a `VP8L` bitstream).
    pub alpha: bool,
    /// The file contains `EXIF` metadata.
    pub exif_metadata: bool,
    /// The file contains `XMP ` metadata.
    pub xmp_metadata: bool,
    /// The image is animated (`ANIM`/`ANMF` chunks).
    pub animation: bool,
    /// Canvas width in pixels (1-based; `1..=2^24`).
    pub canvas_width: u32,
    /// Canvas height in pixels (1-based; `1..=2^24`).
    pub canvas_height: u32,
}

impl Vp8xHeader {
    /// Encodes the 10-byte `VP8X` chunk payload (RFC 9649 §2.7, Figure 7): the feature-flag byte,
    /// three reserved bytes, and the 24-bit little-endian canvas width-minus-one and height-minus-one.
    #[must_use]
    pub fn to_payload(&self) -> [u8; VP8X_PAYLOAD_LEN] {
        let flags = (u8::from(self.icc_profile) << 5)
            | (u8::from(self.alpha) << 4)
            | (u8::from(self.exif_metadata) << 3)
            | (u8::from(self.xmp_metadata) << 2)
            | (u8::from(self.animation) << 1);
        let w = self.canvas_width.saturating_sub(1);
        let h = self.canvas_height.saturating_sub(1);
        [
            flags,
            0,
            0,
            0,
            w as u8,
            (w >> 8) as u8,
            (w >> 16) as u8,
            h as u8,
            (h >> 8) as u8,
            (h >> 16) as u8,
        ]
    }

    /// Parses a `VP8X` chunk payload, mirroring [`to_payload`](Self::to_payload). The two reserved
    /// fields are ignored as the spec requires.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `payload` is shorter than [`VP8X_PAYLOAD_LEN`].
    pub fn from_payload(payload: &[u8]) -> Result<Self> {
        if payload.len() < VP8X_PAYLOAD_LEN {
            return Err(Error::InvalidInput(
                "VP8X: chunk payload shorter than 10 bytes",
            ));
        }
        let flags = payload[0];
        let le24 = |b: &[u8]| u32::from(b[0]) | (u32::from(b[1]) << 8) | (u32::from(b[2]) << 16);
        Ok(Self {
            icc_profile: flags & 0x20 != 0,
            alpha: flags & 0x10 != 0,
            exif_metadata: flags & 0x08 != 0,
            xmp_metadata: flags & 0x04 != 0,
            animation: flags & 0x02 != 0,
            canvas_width: le24(&payload[4..7]) + 1,
            canvas_height: le24(&payload[7..10]) + 1,
        })
    }
}

/// Writes an extended WebP file: the `RIFF`/`WEBP` header, a `VP8X` feature header, then the given
/// chunks in order (RFC 9649 §2.7). Chunk ordering (e.g. `ALPH` before the `VP8 ` bitstream) is the
/// caller's responsibility.
#[must_use]
pub fn write_extended(header: &Vp8xHeader, chunks: &[(FourCc, &[u8])]) -> Vec<u8> {
    let mut w = RiffWriter::new();
    w.write_chunk(FourCc::VP8X, &header.to_payload());
    for (fourcc, payload) in chunks {
        w.write_chunk(*fourcc, payload);
    }
    w.finish()
}

/// The metadata chunks an extended WebP file may carry, **borrowed** rather than copied: the `ICCP`
/// colour profile and the `EXIF` / `XMP ` metadata payloads (RFC 9649 §2.7.2-§2.7.3).
///
/// The container assigns these payloads no meaning — each is carried verbatim, so metadata survives
/// a read/write cycle byte for byte with no reserialization. Use [`MetadataChunks::read`] to collect
/// them from a file and [`write_extended_with_metadata`] to emit them in the spec's chunk order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MetadataChunks<'a> {
    /// The `ICCP` chunk payload: an ICC colour profile. `None` means sRGB is assumed (§2.7.2).
    pub icc: Option<&'a [u8]>,
    /// The `EXIF` chunk payload: Exif metadata, carried bare (no `"Exif\0\0"` signature).
    pub exif: Option<&'a [u8]>,
    /// The `XMP ` chunk payload: an XMP packet.
    pub xmp: Option<&'a [u8]>,
}

impl<'a> MetadataChunks<'a> {
    /// Collects the metadata chunks of the WebP file in `data`, borrowing each payload in place.
    ///
    /// The spec allows at most one chunk of each kind and lets readers "ignore all except the first
    /// one" (RFC 9649 §2.7.2-§2.7.3), so the **first** `ICCP` / `EXIF` / `XMP ` chunk wins. The
    /// `VP8X` feature flags are advisory here: a payload is reported because its chunk is present,
    /// never because a flag claims it is — so a flag set over a missing chunk yields `None`, and a
    /// chunk a non-conformant writer left unflagged is still recovered.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `data` is not a valid RIFF/WebP file, or if a chunk's
    /// declared size runs past the end of the data.
    pub fn read(data: &'a [u8]) -> Result<Self> {
        let mut found = Self::default();
        for chunk in RiffReader::new(data)? {
            let chunk = chunk?;
            let slot = match WebpChunkId::from(chunk.fourcc) {
                WebpChunkId::Iccp => &mut found.icc,
                WebpChunkId::Exif => &mut found.exif,
                WebpChunkId::Xmp => &mut found.xmp,
                _ => continue,
            };
            slot.get_or_insert(chunk.payload);
        }
        Ok(found)
    }

    /// Whether no metadata chunk is present — i.e. nothing here forces a simple (single-bitstream)
    /// file into the extended format.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.icc.is_none() && self.exif.is_none() && self.xmp.is_none()
    }
}

/// Writes an extended WebP file carrying `metadata`, placing every chunk in the canonical order the
/// spec mandates: `VP8X`, `ICCP`, the image data (an optional `ALPH` then the `VP8 `/`VP8L`
/// bitstream), then `EXIF` and `XMP ` (RFC 9649 §2.7 — readers "SHOULD fail" when the chunks needed
/// for reconstruction and colour correction are out of order, and metadata follows the image data).
///
/// The three metadata feature flags of `header` are **derived** from `metadata`, so a chunk can
/// never be emitted without its flag nor a flag without its chunk; `alpha`, `animation`, and the
/// canvas size are taken as given. Ordering *within* `image_data` is the caller's responsibility, as
/// in [`write_extended`].
#[must_use]
pub fn write_extended_with_metadata(
    header: &Vp8xHeader,
    metadata: &MetadataChunks<'_>,
    image_data: &[(FourCc, &[u8])],
) -> Vec<u8> {
    let header = Vp8xHeader {
        icc_profile: metadata.icc.is_some(),
        exif_metadata: metadata.exif.is_some(),
        xmp_metadata: metadata.xmp.is_some(),
        ..*header
    };
    let mut chunks: Vec<(FourCc, &[u8])> = Vec::with_capacity(image_data.len() + 3);
    if let Some(icc) = metadata.icc {
        chunks.push((FourCc::ICCP, icc));
    }
    chunks.extend_from_slice(image_data);
    if let Some(exif) = metadata.exif {
        chunks.push((FourCc::EXIF, exif));
    }
    if let Some(xmp) = metadata.xmp {
        chunks.push((FourCc::XMP, xmp));
    }
    write_extended(&header, &chunks)
}

/// Identifies a WebP chunk by its FourCC, distinguishing the chunks defined by the WebP container
/// spec from any unrecognized ("unknown") chunk that readers must ignore (RFC 9649 §2.5-§2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum WebpChunkId {
    /// Lossy VP8 bitstream (`VP8 `).
    Vp8,
    /// Lossless VP8L bitstream (`VP8L`).
    Vp8l,
    /// Extended-format feature header (`VP8X`).
    Vp8x,
    /// Alpha bitstream (`ALPH`).
    Alpha,
    /// ICC color profile (`ICCP`).
    Iccp,
    /// Exif metadata (`EXIF`).
    Exif,
    /// XMP metadata (`XMP `).
    Xmp,
    /// Global animation parameters (`ANIM`).
    Anim,
    /// Animation frame (`ANMF`).
    Anmf,
    /// A chunk whose FourCC is not one defined by the WebP container spec.
    Unknown(FourCc),
}

impl From<FourCc> for WebpChunkId {
    fn from(fourcc: FourCc) -> Self {
        match &fourcc.0 {
            b"VP8 " => Self::Vp8,
            b"VP8L" => Self::Vp8l,
            b"VP8X" => Self::Vp8x,
            b"ALPH" => Self::Alpha,
            b"ICCP" => Self::Iccp,
            b"EXIF" => Self::Exif,
            b"XMP " => Self::Xmp,
            b"ANIM" => Self::Anim,
            b"ANMF" => Self::Anmf,
            _ => Self::Unknown(fourcc),
        }
    }
}

/// Wraps a VP8L lossless bitstream in the simple WebP (lossless) file format: a `RIFF`/`WEBP` header
/// and a single `VP8L` chunk (RFC 9649 §2.6).
#[must_use]
pub fn write_simple_lossless(vp8l_bitstream: &[u8]) -> Vec<u8> {
    let mut w = RiffWriter::new();
    w.write_chunk(FourCc::VP8L, vp8l_bitstream);
    w.finish()
}

/// Wraps a VP8 lossy bitstream in the simple WebP (lossy) file format: a `RIFF`/`WEBP` header and a
/// single `VP8 ` chunk (RFC 9649 §2.5).
#[must_use]
pub fn write_simple_lossy(vp8_bitstream: &[u8]) -> Vec<u8> {
    let mut w = RiffWriter::new();
    w.write_chunk(FourCc::VP8, vp8_bitstream);
    w.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_known_and_unknown_chunks() {
        assert_eq!(WebpChunkId::from(FourCc::VP8), WebpChunkId::Vp8);
        assert_eq!(WebpChunkId::from(FourCc::VP8L), WebpChunkId::Vp8l);
        assert_eq!(WebpChunkId::from(FourCc::VP8X), WebpChunkId::Vp8x);
        assert_eq!(WebpChunkId::from(FourCc::ALPH), WebpChunkId::Alpha);
        assert_eq!(WebpChunkId::from(FourCc::ICCP), WebpChunkId::Iccp);
        assert_eq!(WebpChunkId::from(FourCc::EXIF), WebpChunkId::Exif);
        assert_eq!(WebpChunkId::from(FourCc::XMP), WebpChunkId::Xmp);
        assert_eq!(WebpChunkId::from(FourCc::ANIM), WebpChunkId::Anim);
        assert_eq!(WebpChunkId::from(FourCc::ANMF), WebpChunkId::Anmf);
        let weird = FourCc::from(*b"XYZW");
        assert_eq!(WebpChunkId::from(weird), WebpChunkId::Unknown(weird));
    }

    #[test]
    fn simple_lossless_wraps_one_vp8l_chunk() {
        let bitstream = [0x2f, 0xde, 0xad, 0xbe, 0xef];
        let file = write_simple_lossless(&bitstream);
        let chunks: Vec<_> = RiffReader::new(&file)
            .unwrap()
            .map(|c| c.unwrap())
            .collect();
        assert_eq!(chunks.len(), 1);
        assert_eq!(WebpChunkId::from(chunks[0].fourcc), WebpChunkId::Vp8l);
        assert_eq!(chunks[0].payload, &bitstream);
    }

    #[test]
    fn vp8x_header_round_trips() {
        let h = Vp8xHeader {
            icc_profile: false,
            alpha: true,
            exif_metadata: false,
            xmp_metadata: false,
            animation: false,
            canvas_width: 640,
            canvas_height: 481,
        };
        let payload = h.to_payload();
        assert_eq!(payload.len(), VP8X_PAYLOAD_LEN);
        assert_eq!(payload[0] & 0x10, 0x10, "alpha (L) flag is bit 4");
        assert_eq!(&payload[1..4], &[0, 0, 0], "reserved bytes are zero");
        assert_eq!(Vp8xHeader::from_payload(&payload).unwrap(), h);
    }

    #[test]
    fn vp8x_all_flags_and_large_canvas_round_trip() {
        // Every feature flag set and a canvas large enough that both 24-bit dimensions use all three
        // bytes — the existing round-trip only sets `alpha` and a sub-2^16 canvas, so the other
        // flags' shifts/masks and the high dimension byte (`>> 16`) went unexercised.
        let h = Vp8xHeader {
            icc_profile: true,
            alpha: true,
            exif_metadata: true,
            xmp_metadata: true,
            animation: true,
            canvas_width: 0x12_3456 + 1,
            canvas_height: 0x65_4321 + 1,
        };
        let p = h.to_payload();
        // flags = icc(0x20) | alpha(0x10) | exif(0x08) | xmp(0x04) | anim(0x02).
        assert_eq!(p[0], 0x3E);
        // 24-bit little-endian width-1 then height-1.
        assert_eq!(&p[4..7], &[0x56, 0x34, 0x12]);
        assert_eq!(&p[7..10], &[0x21, 0x43, 0x65]);
        assert_eq!(Vp8xHeader::from_payload(&p).unwrap(), h);
    }

    #[test]
    fn vp8x_no_flags_round_trips() {
        // All flags clear: each `flags & MASK` test (notably `alpha`, the one set above) is exercised
        // in its *false* state, so a mask mutated to `|` (always-set) is caught.
        let h = Vp8xHeader {
            canvas_width: 1,
            canvas_height: 1,
            ..Default::default()
        };
        let p = h.to_payload();
        assert_eq!(p[0], 0x00);
        assert_eq!(Vp8xHeader::from_payload(&p).unwrap(), h);
    }

    #[test]
    fn from_payload_rejects_short_input() {
        assert!(Vp8xHeader::from_payload(&[0u8; 9]).is_err());
    }

    #[test]
    fn write_extended_assembles_vp8x_then_chunks() {
        let h = Vp8xHeader {
            alpha: true,
            canvas_width: 16,
            canvas_height: 16,
            ..Default::default()
        };
        let file = write_extended(
            &h,
            &[
                (FourCc::ALPH, &[1, 2, 3]),
                (FourCc::VP8, &[0x9d, 0x01, 0x2a]),
            ],
        );
        let chunks: Vec<_> = RiffReader::new(&file)
            .unwrap()
            .map(|c| c.unwrap())
            .collect();
        assert_eq!(WebpChunkId::from(chunks[0].fourcc), WebpChunkId::Vp8x);
        assert_eq!(WebpChunkId::from(chunks[1].fourcc), WebpChunkId::Alpha);
        assert_eq!(WebpChunkId::from(chunks[2].fourcc), WebpChunkId::Vp8);
        assert_eq!(Vp8xHeader::from_payload(chunks[0].payload).unwrap(), h);
    }

    /// The chunk FourCCs of `file`, in file order.
    fn chunk_ids(file: &[u8]) -> Vec<FourCc> {
        RiffReader::new(file)
            .unwrap()
            .map(|c| c.unwrap().fourcc)
            .collect()
    }

    /// The parsed `VP8X` feature header of an extended `file` (its first chunk).
    fn vp8x_header_of(file: &[u8]) -> Vp8xHeader {
        let first = RiffReader::new(file).unwrap().next().unwrap().unwrap();
        assert_eq!(
            first.fourcc,
            FourCc::VP8X,
            "an extended file opens with VP8X"
        );
        Vp8xHeader::from_payload(first.payload).unwrap()
    }

    #[test]
    fn metadata_chunks_round_trip_in_canonical_order() {
        // The spec's order for a still image: VP8X, ICCP, image data, EXIF, XMP — and every payload
        // comes back byte-for-byte (odd lengths included, so the RIFF pad byte is not absorbed).
        let (icc, exif, xmp) = (&[1u8, 2, 3][..], &[0xee; 4][..], &b"<x:xmpmeta/>"[..]);
        let header = Vp8xHeader {
            alpha: true,
            canvas_width: 16,
            canvas_height: 8,
            ..Default::default()
        };
        let metadata = MetadataChunks {
            icc: Some(icc),
            exif: Some(exif),
            xmp: Some(xmp),
        };
        let file = write_extended_with_metadata(
            &header,
            &metadata,
            &[(FourCc::ALPH, &[9, 9]), (FourCc::VP8, &[0x9d, 0x01, 0x2a])],
        );
        assert_eq!(
            chunk_ids(&file),
            vec![
                FourCc::VP8X,
                FourCc::ICCP,
                FourCc::ALPH,
                FourCc::VP8,
                FourCc::EXIF,
                FourCc::XMP,
            ]
        );
        assert_eq!(MetadataChunks::read(&file).unwrap(), metadata);
    }

    #[test]
    fn write_extended_with_metadata_derives_the_feature_flags() {
        // Each metadata flag follows its payload, not the caller's header, in **both** directions: a
        // stale flag with no payload is cleared, and a payload the caller forgot to flag is still
        // advertised. `alpha`, `animation`, and the canvas pass through untouched — the two headers
        // below differ only in the three metadata flags, so comparing whole headers pins that too.
        let all_flags = Vp8xHeader {
            icc_profile: true,
            exif_metadata: true,
            xmp_metadata: true,
            alpha: true,
            animation: true,
            canvas_width: 4,
            canvas_height: 5,
        };
        let no_flags = Vp8xHeader {
            icc_profile: false,
            exif_metadata: false,
            xmp_metadata: false,
            ..all_flags
        };
        let all_payloads = MetadataChunks {
            icc: Some(&[1]),
            exif: Some(&[2]),
            xmp: Some(&[3]),
        };
        let image: &[(FourCc, &[u8])] = &[(FourCc::VP8L, &[0x2f])];

        let cleared = write_extended_with_metadata(&all_flags, &MetadataChunks::default(), image);
        assert_eq!(
            vp8x_header_of(&cleared),
            no_flags,
            "a stale flag must be cleared when nothing is embedded"
        );
        assert_eq!(
            chunk_ids(&cleared),
            vec![FourCc::VP8X, FourCc::VP8L],
            "a stale flag must not conjure an empty chunk"
        );

        let advertised = write_extended_with_metadata(&no_flags, &all_payloads, image);
        assert_eq!(
            vp8x_header_of(&advertised),
            all_flags,
            "an embedded payload must be advertised even if the caller left its flag unset"
        );
    }

    #[test]
    fn write_extended_with_metadata_flags_each_chunk_independently() {
        // One payload at a time: only that chunk's flag may be set, so the three assignments cannot be
        // swapped or share a source.
        let base = Vp8xHeader {
            canvas_width: 2,
            canvas_height: 2,
            ..Default::default()
        };
        let image: &[(FourCc, &[u8])] = &[(FourCc::VP8L, &[0x2f])];
        for (chunks, want) in [
            (
                MetadataChunks {
                    icc: Some(&[1]),
                    ..Default::default()
                },
                Vp8xHeader {
                    icc_profile: true,
                    ..base
                },
            ),
            (
                MetadataChunks {
                    exif: Some(&[2]),
                    ..Default::default()
                },
                Vp8xHeader {
                    exif_metadata: true,
                    ..base
                },
            ),
            (
                MetadataChunks {
                    xmp: Some(&[3]),
                    ..Default::default()
                },
                Vp8xHeader {
                    xmp_metadata: true,
                    ..base
                },
            ),
        ] {
            let file = write_extended_with_metadata(&base, &chunks, image);
            assert_eq!(vp8x_header_of(&file), want, "flags for {chunks:?}");
        }
    }

    #[test]
    fn metadata_chunks_default_is_empty_and_writes_no_metadata() {
        // With nothing to embed, the extended file is exactly `write_extended`'s output — the
        // pre-metadata byte stream is preserved.
        let empty = MetadataChunks::default();
        assert!(empty.is_empty());
        let header = Vp8xHeader {
            alpha: true,
            canvas_width: 2,
            canvas_height: 2,
            ..Default::default()
        };
        let image: &[(FourCc, &[u8])] = &[(FourCc::ALPH, &[1]), (FourCc::VP8, &[2])];
        assert_eq!(
            write_extended_with_metadata(&header, &empty, image),
            write_extended(&header, image)
        );
    }

    #[test]
    fn is_empty_is_false_for_each_chunk_kind() {
        // Each field must count towards emptiness on its own, so a promotion decision keyed on
        // `is_empty` cannot miss a single-chunk file.
        for chunks in [
            MetadataChunks {
                icc: Some(&[0]),
                ..Default::default()
            },
            MetadataChunks {
                exif: Some(&[0]),
                ..Default::default()
            },
            MetadataChunks {
                xmp: Some(&[0]),
                ..Default::default()
            },
        ] {
            assert!(!chunks.is_empty(), "{chunks:?} carries a payload");
        }
    }

    #[test]
    fn metadata_chunks_read_keeps_the_first_of_each_kind() {
        // "Readers MAY ignore all except the first one" (§2.7.2-§2.7.3).
        let mut w = RiffWriter::new();
        w.write_chunk(FourCc::ICCP, b"first-icc");
        w.write_chunk(FourCc::VP8L, &[0x2f]);
        w.write_chunk(FourCc::EXIF, b"first-exif");
        w.write_chunk(FourCc::EXIF, b"second-exif");
        w.write_chunk(FourCc::ICCP, b"second-icc");
        let file = w.finish();
        let got = MetadataChunks::read(&file).unwrap();
        assert_eq!(got.icc, Some(&b"first-icc"[..]));
        assert_eq!(got.exif, Some(&b"first-exif"[..]));
        assert_eq!(got.xmp, None);
    }

    #[test]
    fn metadata_chunks_read_reports_presence_not_vp8x_flags() {
        // A flag set over a missing chunk yields `None`; a chunk an unconformant writer left
        // unflagged is still recovered. The flags never fabricate or suppress a payload.
        let lying = Vp8xHeader {
            icc_profile: true,
            exif_metadata: true,
            xmp_metadata: true,
            canvas_width: 1,
            canvas_height: 1,
            ..Default::default()
        };
        let mut w = RiffWriter::new();
        w.write_chunk(FourCc::VP8X, &lying.to_payload());
        w.write_chunk(FourCc::VP8L, &[0x2f]);
        w.write_chunk(FourCc::XMP, b"<x/>");
        let file = w.finish();
        assert_eq!(
            MetadataChunks::read(&file).unwrap(),
            MetadataChunks {
                icc: None,
                exif: None,
                xmp: Some(&b"<x/>"[..]),
            }
        );
    }

    #[test]
    fn metadata_chunks_read_is_empty_for_a_simple_file() {
        let file = write_simple_lossless(&[0x2f, 1, 2]);
        assert!(MetadataChunks::read(&file).unwrap().is_empty());
    }

    #[test]
    fn metadata_chunks_read_rejects_malformed_input() {
        assert!(MetadataChunks::read(b"not a webp file").is_err());
        // A chunk whose declared size runs past the data must surface the reader's error rather than
        // being silently treated as "no metadata".
        let mut file = write_simple_lossless(&[0; 4]);
        file[16..20].copy_from_slice(&5u32.to_le_bytes());
        assert!(MetadataChunks::read(&file).is_err());
    }

    #[test]
    fn simple_lossy_wraps_one_vp8_chunk() {
        let bitstream = [0x9d, 0x01, 0x2a];
        let file = write_simple_lossy(&bitstream);
        let chunks: Vec<_> = RiffReader::new(&file)
            .unwrap()
            .map(|c| c.unwrap())
            .collect();
        assert_eq!(chunks.len(), 1);
        assert_eq!(WebpChunkId::from(chunks[0].fourcc), WebpChunkId::Vp8);
        assert_eq!(chunks[0].payload, &bitstream);
    }
}
