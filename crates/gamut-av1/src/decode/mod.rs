//! AV1 still-image decoder (AV1 §5 parsing, §7 decoding process).
//!
//! The mirror of this crate's encoder, and the pure-Rust software tail behind
//! `gamut_avif::Av1StillDecoder`. Where the encoder chooses a subset of AV1's tools, a decoder
//! must accept whatever a conformant encoder produced — so these modules follow the spec's syntax
//! tables rather than the encoder's choices, and refuse, with a typed
//! [`Error::Unsupported`](gamut_core::Error::Unsupported), any tool that is signalled but not yet
//! implemented. A refusal is always explicit; nothing is approximated.
//!
//! Modules mirror the spec:
//!
//! - [`obu`] — OBU framing (§5.3) and the sequence header (§5.5).
//! - [`header`] — the uncompressed frame header (§5.9).
//! - [`tilegroup`] — the frame OBU and tile group framing (§5.10, §5.11.1).
//!
//! Reconstruction reuses the normative machinery the encoder already carries: `transform`'s
//! [`inverse_transform_2d`](crate::transform::inverse_transform_2d), `quant`'s
//! [`dequant`](crate::quant::dequant), the `cdf` tables, and `filter`'s in-loop filters.
//!
//! **Implemented scope.** `seq_profile = 1` (8-bit 4:4:4) intra key frames. The pixel-format
//! matrix (10/12-bit, 4:2:0/4:2:2, monochrome), intra block copy, and film grain are refused;
//! each is a ☐ row in `gamut-avif/STATUS.md`.

use gamut_bitstream::BitReader;
use gamut_core::{Error, Result};

pub(crate) mod header;
pub(crate) mod obu;
pub(crate) mod tilegroup;

/// Origin tag on every error the decoder raises.
pub(crate) const ORIGIN: &str = "gamut-av1";

pub use header::{
    CdefParams, FrameHeader, LoopFilterParams, LrParams, QuantizationParams, RestorationType,
    SegmentationParams, TileInfo, TxMode,
};
pub use obu::{ColorConfig, SequenceHeader, Subsampling};

/// Resource ceilings applied before any allocation sized by the bitstream.
///
/// A decoder is the format's attack surface: every buffer it allocates is sized by numbers an
/// attacker chose. These caps are checked against the *header* before a single sample is
/// allocated, so a hostile stream is refused rather than serviced. The defaults are generous for
/// real still images and far below what would exhaust a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeLimits {
    /// Largest accepted luma width, in samples.
    pub max_width: u32,
    /// Largest accepted luma height, in samples.
    pub max_height: u32,
    /// Largest accepted luma sample count (`width * height`), bounding the total allocation
    /// independently of the aspect ratio.
    pub max_pixels: u64,
    /// Largest accepted number of tiles (`TileCols * TileRows`).
    pub max_tiles: usize,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            // AV1 level 6.0 tops out at 16384 in either dimension.
            max_width: 16384,
            max_height: 16384,
            // 256 Mpx: a 3-plane 8-bit frame held as u16 is then ~1.5 GiB at the very top end,
            // and typical stills are three orders of magnitude below it.
            max_pixels: 256 << 20,
            // MAX_TILE_ROWS * MAX_TILE_COLS (§3).
            max_tiles: 64 * 64,
        }
    }
}

impl DecodeLimits {
    /// Checks a frame's declared geometry against the caps.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if the frame exceeds any cap.
    pub fn check(&self, width: u32, height: u32, tiles: usize) -> Result<()> {
        if width == 0 || height == 0 {
            return Err(Error::invalid_input(
                ORIGIN,
                "AV1 decode: frame has a zero dimension",
            ));
        }
        if width > self.max_width || height > self.max_height {
            return Err(Error::invalid_input(
                ORIGIN,
                "AV1 decode: frame dimensions exceed the configured limit",
            ));
        }
        if u64::from(width) * u64::from(height) > self.max_pixels {
            return Err(Error::invalid_input(
                ORIGIN,
                "AV1 decode: frame sample count exceeds the configured limit",
            ));
        }
        if tiles > self.max_tiles {
            return Err(Error::invalid_input(
                ORIGIN,
                "AV1 decode: tile count exceeds the configured limit",
            ));
        }
        Ok(())
    }
}

/// The headers of an AV1 still, without decoding any samples.
///
/// What a container needs to cross-check its own records against the codestream — the shape
/// `gamut-avif` re-derives `av1C`/`colr` from (AV1-ISOBMFF §2.3.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamInfo {
    /// The sequence header (§5.5.1).
    pub sequence: SequenceHeader,
    /// The frame header (§5.9.2).
    pub frame: FrameHeader,
    /// The number of tiles the frame's tile group carries.
    pub tile_count: usize,
}

/// The pure-Rust AV1 still-image decoder.
///
/// Reads one AV1 temporal unit — a sequence header plus an intra key frame. This is the software
/// tail behind `gamut_avif`'s decoder seam, and is usable standalone for a bare AV1 still
/// bitstream.
///
/// # Implemented scope
///
/// Today this decodes the **framing and header layer**: OBU walk (§5.3), the full sequence header
/// (§5.5), the full uncompressed frame header (§5.9), and tile-group framing (§5.11.1), surfaced
/// by [`Av1Decoder::inspect`]. Sample decoding — the tile body, reconstruction and the in-loop
/// filters — is not here yet and no entry point pretends otherwise; it arrives in the next slices
/// of issue #259.
///
/// Streams are accepted only at `seq_profile = 1` (8-bit 4:4:4) with intra key frames. Anything
/// else is refused with a typed [`Error::Unsupported`](gamut_core::Error::Unsupported) naming the
/// tool — never approximated. The remaining surface is tracked row by row in
/// `gamut-avif/STATUS.md`.
#[derive(Debug, Clone, Default)]
pub struct Av1Decoder {
    /// Resource ceilings applied before any bitstream-sized allocation.
    limits: DecodeLimits,
}

impl Av1Decoder {
    /// Creates a decoder with the default [`DecodeLimits`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a decoder with caller-chosen resource ceilings.
    #[must_use]
    pub const fn with_limits(limits: DecodeLimits) -> Self {
        Self { limits }
    }

    /// The ceilings this decoder enforces.
    #[must_use]
    pub const fn limits(&self) -> &DecodeLimits {
        &self.limits
    }

    /// Parses the sequence and frame headers of a still without decoding any samples.
    ///
    /// Cheap enough to run before committing to a decode: it walks the OBUs, reads both headers,
    /// applies this decoder's [`DecodeLimits`], refuses unimplemented tools, and validates the
    /// tile framing — but never touches a tile body. Use it to check a container's records
    /// against the codestream, or to learn a still's geometry and colour signalling up front.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`](gamut_core::Error::InvalidInput) if the bitstream is
    /// malformed, truncated, or exceeds this decoder's [`DecodeLimits`], and
    /// [`Error::Unsupported`](gamut_core::Error::Unsupported) if it uses a coding tool that is
    /// not implemented.
    pub fn inspect(&self, temporal_unit: &[u8]) -> Result<StreamInfo> {
        let (sequence, frame, tiles) = self.parse_frame(temporal_unit)?;
        Ok(StreamInfo {
            sequence,
            frame,
            tile_count: tiles.len(),
        })
    }

    /// Walks the temporal unit and parses everything up to (but not including) the tile bodies.
    ///
    /// Backs [`Av1Decoder::inspect`], and is where sample decoding will attach once the tile
    /// body lands: it already produces the per-tile byte slices that step consumes.
    fn parse_frame<'a>(
        &self,
        temporal_unit: &'a [u8],
    ) -> Result<(SequenceHeader, FrameHeader, Vec<&'a [u8]>)> {
        let mut seq: Option<SequenceHeader> = None;
        let mut frame: Option<(FrameHeader, Vec<&'a [u8]>)> = None;

        for obu in obu::ObuIter::new(temporal_unit) {
            let obu = obu?;
            // §5.3.1: an OBU carrying an extension header is dropped when the chosen operating
            // point excludes its temporal or spatial layer. Sequence headers and temporal
            // delimiters are never dropped.
            if obu.has_extension
                && obu.kind != obu::OBU_SEQUENCE_HEADER
                && obu.kind != obu::OBU_TEMPORAL_DELIMITER
                && let Some(sh) = &seq
                && sh.operating_point_idc != 0
            {
                let in_temporal = (sh.operating_point_idc >> obu.temporal_id) & 1 != 0;
                let in_spatial = (sh.operating_point_idc >> (obu.spatial_id + 8)) & 1 != 0;
                if !in_temporal || !in_spatial {
                    continue;
                }
            }

            match obu.kind {
                obu::OBU_SEQUENCE_HEADER => {
                    let parsed = SequenceHeader::parse(obu.payload)?;
                    // A repeated sequence header must be identical (AV1 §7.5); a changed one would
                    // reinterpret the frame that follows.
                    if let Some(previous) = seq
                        && previous != parsed
                    {
                        return Err(Error::invalid_input(
                            ORIGIN,
                            "AV1 decode: repeated sequence header differs from the first",
                        ));
                    }
                    seq = Some(parsed);
                }
                obu::OBU_FRAME => {
                    let sh = seq.as_ref().ok_or_else(|| {
                        Error::invalid_input(
                            ORIGIN,
                            "AV1 decode: frame OBU before any sequence header",
                        )
                    })?;
                    if frame.is_some() {
                        return Err(Error::unsupported(
                            ORIGIN,
                            "AV1 decode: a still image must carry exactly one frame",
                        ));
                    }
                    // frame_obu (§5.10): the frame header, byte alignment, then the tile group.
                    let mut r = BitReader::new(obu.payload);
                    let fh = FrameHeader::parse(&mut r, sh)?;
                    r.byte_alignment()?;
                    self.validate(sh, &fh)?;
                    let tiles = tilegroup::split_tiles(r.remaining_bytes(), &fh.tile_info)?;
                    frame = Some((fh, tiles));
                }
                obu::OBU_FRAME_HEADER | obu::OBU_REDUNDANT_FRAME_HEADER => {
                    let sh = seq.as_ref().ok_or_else(|| {
                        Error::invalid_input(
                            ORIGIN,
                            "AV1 decode: frame header OBU before any sequence header",
                        )
                    })?;
                    if obu.kind == obu::OBU_REDUNDANT_FRAME_HEADER {
                        // A redundant copy repeats a header already seen; nothing new to decode.
                        continue;
                    }
                    if frame.is_some() {
                        return Err(Error::unsupported(
                            ORIGIN,
                            "AV1 decode: a still image must carry exactly one frame",
                        ));
                    }
                    let mut r = BitReader::new(obu.payload);
                    let fh = FrameHeader::parse(&mut r, sh)?;
                    r.trailing_bits()?;
                    self.validate(sh, &fh)?;
                    // The tiles arrive in a separate OBU_TILE_GROUP; record the header and wait.
                    frame = Some((fh, Vec::new()));
                }
                obu::OBU_TILE_GROUP => {
                    let (fh, tiles) = frame.as_mut().ok_or_else(|| {
                        Error::invalid_input(
                            ORIGIN,
                            "AV1 decode: tile group OBU before any frame header",
                        )
                    })?;
                    if !tiles.is_empty() {
                        return Err(Error::unsupported(
                            ORIGIN,
                            "AV1 decode: a still image must carry every tile in one tile group",
                        ));
                    }
                    *tiles = tilegroup::split_tiles(obu.payload, &fh.tile_info)?;
                }
                obu::OBU_TILE_LIST => {
                    return Err(Error::invalid_input(
                        ORIGIN,
                        "AV1 decode: tile list OBUs are forbidden in a still image",
                    ));
                }
                // Temporal delimiters, metadata, padding, and reserved types are ignored (§5.3.1).
                obu::OBU_TEMPORAL_DELIMITER | obu::OBU_METADATA | obu::OBU_PADDING => {}
                _ => {}
            }
        }

        let seq = seq.ok_or_else(|| {
            Error::invalid_input(
                ORIGIN,
                "AV1 decode: no sequence header in the temporal unit",
            )
        })?;
        let (frame, tiles) = frame.ok_or_else(|| {
            Error::invalid_input(ORIGIN, "AV1 decode: no frame in the temporal unit")
        })?;
        if tiles.is_empty() {
            return Err(Error::invalid_input(
                ORIGIN,
                "AV1 decode: frame header with no tile data",
            ));
        }
        Ok((seq, frame, tiles))
    }

    /// Applies the resource ceilings and the unsupported-tool refusals to a parsed header.
    fn validate(&self, seq: &SequenceHeader, frame: &FrameHeader) -> Result<()> {
        self.limits.check(
            frame.upscaled_width,
            frame.frame_height,
            frame.tile_info.tile_cols * frame.tile_info.tile_rows,
        )?;
        frame.reject_unsupported_tools(seq)
    }
}

/// Shared fixtures for the decode-side unit tests.
#[cfg(test)]
pub(crate) mod testutil {
    use crate::headers::{Av1Colour, Av1StillConfig};

    /// Builds the [`Av1StillConfig`] the encoder derives for `colour` at this size, so the
    /// parsers here are checked against the exact headers `gamut-av1` emits.
    pub(crate) fn still_config(width: u32, height: u32, colour: Av1Colour) -> Av1StillConfig {
        use gamut_color::cicp::ColorRange;
        Av1StillConfig {
            seq_profile: 1,
            seq_level_idx_0: crate::headers::pick_level(width, height).unwrap(),
            seq_tier_0: 0,
            high_bitdepth: false,
            twelve_bit: false,
            monochrome: false,
            chroma_subsampling_x: 0,
            chroma_subsampling_y: 0,
            chroma_sample_position: 0,
            color_primaries: colour.primaries.code_point(),
            transfer_characteristics: colour.transfer.code_point(),
            matrix_coefficients: colour.matrix.code_point(),
            full_range: matches!(colour.range, ColorRange::Full),
        }
    }

    /// Builds the sequence-header OBU payload the encoder emits, so the parser is checked against
    /// the writer it mirrors.
    pub(crate) fn encoder_seq_header(
        width: u32,
        height: u32,
        lossy: bool,
        superres: bool,
    ) -> Vec<u8> {
        let cfg = still_config(width, height, Av1Colour::default());
        crate::headers::sequence_header_payload(&cfg, width, height, lossy, superres)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_reject_degenerate_and_oversized_frames() {
        let limits = DecodeLimits::default();
        limits.check(1920, 1080, 1).unwrap();

        assert_eq!(
            limits.check(0, 16, 1).unwrap_err().static_message(),
            Some("AV1 decode: frame has a zero dimension")
        );
        assert_eq!(
            limits.check(16, 0, 1).unwrap_err().static_message(),
            Some("AV1 decode: frame has a zero dimension")
        );
        assert_eq!(
            limits.check(16385, 16, 1).unwrap_err().static_message(),
            Some("AV1 decode: frame dimensions exceed the configured limit")
        );
        assert_eq!(
            limits.check(16, 16385, 1).unwrap_err().static_message(),
            Some("AV1 decode: frame dimensions exceed the configured limit")
        );
        assert_eq!(
            limits.check(16, 16, 4097).unwrap_err().static_message(),
            Some("AV1 decode: tile count exceeds the configured limit")
        );
    }

    #[test]
    fn the_pixel_cap_binds_independently_of_the_dimension_caps() {
        // Both dimensions are inside their own cap, but the product is not.
        let limits = DecodeLimits {
            max_pixels: 1024,
            ..DecodeLimits::default()
        };
        limits.check(32, 32, 1).unwrap();
        assert_eq!(
            limits.check(64, 32, 1).unwrap_err().static_message(),
            Some("AV1 decode: frame sample count exceeds the configured limit")
        );
    }
}
