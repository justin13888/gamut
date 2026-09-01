//! Where a PNG's bytes went (issue #224): every byte of the file classified into a typed
//! [`Segment`], plus the per-stage figures an encoder-efficiency comparison is built from.
//!
//! This is the measurement counterpart to [`crate::PngEncoder`]. It works on **any** PNG, not
//! just this crate's output, so the same numbers can be read off libpng's, oxipng's or
//! zopflipng's files and compared directly: bits per pixel, what the DEFLATE stage achieved in
//! isolation, how many bytes went to chunk framing, and which scanline filters the encoder
//! actually chose.
//!
//! # The every-byte invariant
//!
//! [`PngReport::segments`] is contiguous, non-overlapping, and covers `0..file_len` exactly.
//! It holds by construction, and [`PngReport::is_fully_classified`] re-derives it from the list
//! rather than storing a flag, so a walk bug makes the predicate false instead of silently
//! agreeing with itself. This mirrors [`gamut_isobmff::segments`]'s guarantee for ISOBMFF; PNG's
//! chunk stream needs its own walk (there are no boxes and no `meta` level), but the shape and
//! the names are deliberately the same.
//!
//! # What is an error and what is a finding
//!
//! Deliberately more tolerant than [`crate::PngDecoder::metadata`], and for the reason
//! [`gamut_dng::deconstruct`] gives: a measurement tool that refuses to measure is useless.
//! Unknown ancillary **and critical** chunks, CRC mismatches, a missing IEND, trailing bytes and
//! a truncated tail are all *reported*, never errors. Only a file with no header to report on —
//! bad signature, no first chunk, a first chunk that is not IHDR, or an unparsable IHDR — fails.

use core::ops::Range;

use gamut_core::{Error, Result};

use crate::chunk::{ChunkReader, RawChunk, SIGNATURE};
use crate::decoded::PngHeader;
use crate::filter::FilterType;
use crate::{adam7, ihdr, inflate};

/// Chunk framing overhead: 4 length bytes + 4 type bytes + 4 CRC bytes (§5.3).
const FRAMING: usize = 12;

/// One contiguous run of the input file, tagged by what it holds ([`SegmentKind`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// The half-open byte range this segment occupies within the input (`start..end`).
    pub range: Range<usize>,
    /// What the bytes in [`range`](Self::range) are.
    pub kind: SegmentKind,
}

/// What a [`Segment`] holds.
///
/// Non-exhaustive: a future revision may name a further region (an APNG frame span, say) without
/// a breaking change — match with a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SegmentKind {
    /// The 8-byte PNG file signature (§5.2). Always the first segment.
    Signature,
    /// One complete chunk: 4 length bytes, 4 type bytes, the payload, 4 CRC bytes (§5.3), so the
    /// segment is always `payload_len + 12` bytes long.
    Chunk {
        /// The chunk's four-character type, e.g. `*b"IDAT"`. Recognised and unrecognised types
        /// alike appear here — critical ones included; the walk never drops a chunk.
        chunk_type: [u8; 4],
        /// The declared payload length, framing excluded.
        payload_len: usize,
        /// Whether the stored CRC-32 over type + payload matched (§5.5). A mismatch is reported,
        /// never an error: §13.1 makes it recoverable in an ancillary chunk, and the framing is
        /// intact either way, so the walk can keep going and account the rest of the file.
        crc_ok: bool,
    },
    /// Bytes after IEND. Not part of the datastream — §13.2 asks decoders to ignore them, so they
    /// are surfaced here rather than silently dropped.
    Trailer,
    /// From the first chunk header that does not frame — truncated, or declaring a length that
    /// overruns the input — to end of file. A file carrying one is not a complete PNG.
    Truncated,
}

/// Per-chunk-type totals, in first-appearance order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ChunkStats {
    /// The chunk's four-character type.
    pub chunk_type: [u8; 4],
    /// How many chunks of this type the file carries.
    pub count: usize,
    /// Total payload bytes across those chunks — framing excluded.
    pub payload_bytes: usize,
}

impl ChunkStats {
    /// Framing bytes these chunks cost: 12 per chunk (4 length + 4 type + 4 CRC, §5.3).
    #[must_use]
    pub fn framing_bytes(&self) -> usize {
        self.count * FRAMING
    }

    /// Payload plus framing — what this chunk type costs the file in total.
    #[must_use]
    pub fn total_bytes(&self) -> usize {
        self.payload_bytes + self.framing_bytes()
    }

    /// Whether the type is ancillary — bit 5 of the first byte set, i.e. lowercase (§5.4).
    #[must_use]
    pub fn is_ancillary(&self) -> bool {
        self.chunk_type[0] & 0x20 != 0
    }
}

/// One reduced image making up the filtered stream: an Adam7 pass (§8.1), or the whole image when
/// the file is not interlaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PassStats {
    /// Pass index in transmission order (`0..7`); always `0` when the file is not interlaced.
    pub index: u8,
    /// The reduced image's width in pixels. Never zero: an empty pass carries no bytes at all,
    /// not even filter-type bytes (§7.3), so it is omitted entirely.
    pub width: u32,
    /// The reduced image's height in pixels. Never zero, for the same reason.
    pub height: u32,
    /// Bytes per scanline excluding the filter-type byte: `ceil(width × bits_per_pixel / 8)`, so
    /// a sub-byte depth includes its row padding (§7.2).
    pub row_bytes: usize,
    /// This pass's contribution to the filtered stream: `height × (1 + row_bytes)`.
    pub filtered_len: usize,
}

/// How many scanlines chose each of the five filters (§9.1), summed over every pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilterHistogram {
    counts: [u32; 5],
}

impl FilterHistogram {
    /// Scanlines that chose `filter`.
    #[must_use]
    pub fn count(self, filter: FilterType) -> u32 {
        self.counts[filter as usize]
    }

    /// Total scanlines — the sum over all five filters, and the image's scanline count.
    #[must_use]
    pub fn total(self) -> u32 {
        self.counts.iter().sum()
    }
}

/// Where a PNG's bytes went: a total byte accounting plus the figures an encoder-efficiency
/// comparison is built from. Produced by [`deconstruct`].
///
/// Non-exhaustive: report categories may be added without a breaking change.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PngReport {
    /// The input's total length in bytes — what [`segments`](Self::segments) together covers.
    pub file_len: usize,
    /// IHDR: dimensions, bit depth, colour type, interlace method.
    pub header: PngHeader,
    /// Every byte of the input in file order — contiguous, non-overlapping, covering
    /// `0..file_len` exactly. See the [every-byte invariant](self#the-every-byte-invariant).
    pub segments: Vec<Segment>,
    /// Per-chunk-type totals, in first-appearance order.
    pub chunks: Vec<ChunkStats>,
    /// The concatenated IDAT payload length: the zlib codestream, framing excluded. This is what
    /// the encoder's compression stage produced, and the numerator of
    /// [`idat_ratio`](Self::idat_ratio).
    pub idat_compressed: usize,
    /// The length that codestream inflates to — the filter-prefixed scanline stream. Derived from
    /// IHDR alone (the sum over [`passes`](Self::passes) when interlaced), so it is known even
    /// when [`filters`](Self::filters) is `None`.
    pub filtered_len: usize,
    /// The reduced images making up the filtered stream: one entry per non-empty Adam7 pass, or
    /// exactly one entry for a non-interlaced image.
    pub passes: Vec<PassStats>,
    /// Scanlines per filter type, or `None` when the IDAT stream was not inflated: it was corrupt
    /// or truncated, it did not inflate to [`filtered_len`](Self::filtered_len), it carried an
    /// undefined filter code, or it was larger than the inflation cap. Everything else in this
    /// report is available without inflating.
    pub filters: Option<FilterHistogram>,
}

impl PngReport {
    /// **The headline law.** Whether the segments tile the input exactly: the first starts at 0,
    /// each ends where the next starts, none is empty, and the last ends at `file_len`.
    /// Re-derived from [`segments`](Self::segments) rather than stored.
    #[must_use]
    pub fn is_fully_classified(&self) -> bool {
        let mut expected = 0usize;
        for segment in &self.segments {
            if segment.range.start != expected || segment.range.end <= segment.range.start {
                return false;
            }
            expected = segment.range.end;
        }
        expected == self.file_len
    }

    /// Whether every byte of this file belongs to a complete, undamaged PNG datastream: fully
    /// classified, no [`SegmentKind::Truncated`] and no [`SegmentKind::Trailer`], every CRC
    /// valid, IEND present, and the IDAT stream inflated to exactly
    /// [`filtered_len`](Self::filtered_len).
    ///
    /// A trailer counts against it even though §13.2 lets a *decoder* ignore trailing bytes,
    /// because [`bits_per_pixel`](Self::bits_per_pixel) divides the whole file by the pixel
    /// count: bytes outside the datastream still inflate the headline figure, so a size
    /// comparison has to know they are there.
    ///
    /// Independent of whether every chunk type was *recognised* — an unknown critical chunk is
    /// still accounted for.
    #[must_use]
    pub fn is_intact(&self) -> bool {
        self.is_fully_classified()
            && self.filters.is_some()
            && self.segments.iter().all(|segment| match segment.kind {
                SegmentKind::Truncated | SegmentKind::Trailer => false,
                SegmentKind::Chunk { crc_ok, .. } => crc_ok,
                SegmentKind::Signature => true,
            })
            && self.chunk(b"IEND").is_some()
    }

    /// **Stored bits per image pixel** — the space-efficiency figure of merit: the whole file,
    /// framing and metadata included, over `width × height`. Distinct from the *uncompressed*
    /// rate, which is `header.color_type.channels() × header.bit_depth`.
    #[must_use]
    pub fn bits_per_pixel(&self) -> f64 {
        let pixels = f64::from(self.header.width) * f64::from(self.header.height);
        // IHDR rejects a zero dimension, so `pixels >= 1.0` for any report that exists.
        self.file_len as f64 * 8.0 / pixels
    }

    /// The DEFLATE stage's compression ratio in isolation: `idat_compressed / filtered_len`.
    /// Below 1.0 means the codestream compressed. Filtering and colour-type choice are *upstream*
    /// of this number, which is what makes it the right lens for attributing a size difference to
    /// the compressor rather than to the rest of the encoder.
    #[must_use]
    pub fn idat_ratio(&self) -> f64 {
        if self.filtered_len == 0 {
            return 0.0;
        }
        self.idat_compressed as f64 / self.filtered_len as f64
    }

    /// Every byte that is not IDAT payload: the signature, all chunk framing, and every non-IDAT
    /// payload.
    #[must_use]
    pub fn overhead_bytes(&self) -> usize {
        self.file_len - self.idat_compressed
    }

    /// Total chunk framing: 12 bytes per chunk in the file.
    #[must_use]
    pub fn framing_bytes(&self) -> usize {
        self.chunks.iter().map(ChunkStats::framing_bytes).sum()
    }

    /// The stats for one chunk type, if the file carries it.
    #[must_use]
    pub fn chunk(&self, chunk_type: &[u8; 4]) -> Option<ChunkStats> {
        self.chunks
            .iter()
            .find(|stats| &stats.chunk_type == chunk_type)
            .copied()
    }
}

/// The largest filtered stream this walk will inflate to count filter choices. Matches the
/// decoder's own default image budget, so a report never allocates more than a decode would.
const MAX_FILTERED_BYTES: usize = 64 << 20;

/// Classifies every byte of `png` and, where the IDAT stream is sound and within budget, counts
/// the scanline filter each row chose.
///
/// Pixels are never reconstructed: no defiltering, no unpacking, no de-interlacing, no palette
/// resolution. The walk reads chunk framing and the IHDR, and inflates IDAT only to read one
/// filter byte per scanline.
///
/// Works on any PNG, whichever encoder produced it, which is what makes the figures comparable
/// across encoders (issue #224).
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] only when there is no header to report on: a bad signature, no
/// first chunk, a first chunk that is not IHDR, or an IHDR whose payload is invalid. Everything
/// else is **reported, not errored** — unknown ancillary *and critical* chunks, CRC mismatches, a
/// missing IEND, trailing bytes after IEND, a truncated tail, and a corrupt IDAT stream.
pub fn deconstruct(png: &[u8]) -> Result<PngReport> {
    let mut reader = ChunkReader::new(png)?;
    let mut segments = vec![Segment {
        range: 0..SIGNATURE.len(),
        kind: SegmentKind::Signature,
    }];

    let first = reader.next_chunk()?.ok_or_else(|| {
        Error::invalid_input(env!("CARGO_PKG_NAME"), "PNG: no chunk after the signature")
    })?;
    if &first.chunk_type != b"IHDR" {
        return Err(Error::invalid_input(
            env!("CARGO_PKG_NAME"),
            "PNG: first chunk is not IHDR",
        ));
    }
    let native = ihdr::parse(first.data)?;
    let header = PngHeader {
        width: native.width,
        height: native.height,
        bit_depth: native.bit_depth,
        color_type: native.color,
        interlaced: native.interlaced,
    };

    let mut chunks: Vec<ChunkStats> = Vec::new();
    let mut idat = Vec::new();
    let mut saw_iend = false;
    let push = |segments: &mut Vec<Segment>, chunks: &mut Vec<ChunkStats>, chunk: &RawChunk| {
        segments.push(Segment {
            range: chunk.range.clone(),
            kind: SegmentKind::Chunk {
                chunk_type: chunk.chunk_type,
                payload_len: chunk.data.len(),
                crc_ok: chunk.crc_ok,
            },
        });
        match chunks
            .iter_mut()
            .find(|stats| stats.chunk_type == chunk.chunk_type)
        {
            Some(stats) => {
                stats.count += 1;
                stats.payload_bytes += chunk.data.len();
            }
            None => chunks.push(ChunkStats {
                chunk_type: chunk.chunk_type,
                count: 1,
                payload_bytes: chunk.data.len(),
            }),
        }
    };
    push(&mut segments, &mut chunks, &first);

    loop {
        match reader.next_chunk() {
            Ok(None) => break,
            Ok(Some(chunk)) => {
                if &chunk.chunk_type == b"IDAT" {
                    idat.extend_from_slice(chunk.data);
                }
                let is_iend = &chunk.chunk_type == b"IEND";
                push(&mut segments, &mut chunks, &chunk);
                if is_iend {
                    saw_iend = true;
                    break;
                }
            }
            // A header that does not frame ends the datastream; the rest of the file is
            // accounted as one opaque run rather than dropped (§13.2's tolerance, extended to
            // damage the spec does not describe).
            Err(_) => {
                let start = reader.offset();
                if start < png.len() {
                    segments.push(Segment {
                        range: start..png.len(),
                        kind: SegmentKind::Truncated,
                    });
                }
                break;
            }
        }
    }
    if saw_iend && reader.offset() < png.len() {
        segments.push(Segment {
            range: reader.offset()..png.len(),
            kind: SegmentKind::Trailer,
        });
    }

    let passes = pass_stats(&native);
    let filtered_len = adam7::expected_stream_len(&native).unwrap_or(0);
    let filters = filter_histogram(&idat, filtered_len, &passes);

    Ok(PngReport {
        file_len: png.len(),
        header,
        segments,
        chunks,
        idat_compressed: idat.len(),
        filtered_len,
        passes,
        filters,
    })
}

/// The reduced images making up the filtered stream, skipping empty passes exactly as
/// [`adam7::expected_stream_len`] does — so `filtered_len` is the sum of these and can be checked
/// against it rather than merely asserted.
fn pass_stats(header: &ihdr::Ihdr) -> Vec<PassStats> {
    let mut out = Vec::new();
    for (index, pass) in adam7::passes_for(header.interlaced).iter().enumerate() {
        let (width, height) = adam7::pass_dimensions(pass, header.width, header.height);
        if width == 0 || height == 0 {
            continue;
        }
        let Some(row_bytes) = (width as usize)
            .checked_mul(header.bits_per_pixel())
            .map(|bits| bits.div_ceil(8))
        else {
            return Vec::new();
        };
        let Some(filtered_len) = row_bytes
            .checked_add(1)
            .and_then(|stride| (height as usize).checked_mul(stride))
        else {
            return Vec::new();
        };
        out.push(PassStats {
            index: index as u8,
            width,
            height,
            row_bytes,
            filtered_len,
        });
    }
    out
}

/// Inflates the IDAT stream and counts the filter byte leading each scanline.
///
/// `None` whenever the count cannot be trusted: the stream is over budget, corrupt, truncated,
/// inflates to the wrong length, or carries a code §9.1 does not define. Every other figure in
/// the report is derived from framing and IHDR, so it survives all of these.
fn filter_histogram(
    idat: &[u8],
    filtered_len: usize,
    passes: &[PassStats],
) -> Option<FilterHistogram> {
    if filtered_len == 0 || filtered_len > MAX_FILTERED_BYTES {
        return None;
    }
    let stream = inflate::inflate_zlib(idat, filtered_len).ok()?;
    if stream.len() != filtered_len {
        return None;
    }
    let mut counts = [0u32; 5];
    let mut at = 0usize;
    for pass in passes {
        for _ in 0..pass.height {
            let filter = FilterType::from_code(*stream.get(at)?)?;
            counts[filter as usize] += 1;
            at += 1 + pass.row_bytes;
        }
    }
    Some(FilterHistogram { counts })
}
