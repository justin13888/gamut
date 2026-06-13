//! WebP-specific helpers over the generic RIFF layer: classifying WebP chunks, the [`Vp8xHeader`]
//! extended-format feature header, and writing the simple (single-bitstream) and extended file
//! formats (RFC 9649 §2.5-§2.7).
//!
//! The remaining extended-format chunks (`ALPH`, `ANIM`/`ANMF`, metadata) are tracked in
//! `gamut-webp/STATUS.md` section A and land with the alpha/animation milestones.

use gamut_core::{Error, Result};

use crate::fourcc::FourCc;
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
    use crate::reader::RiffReader;

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
