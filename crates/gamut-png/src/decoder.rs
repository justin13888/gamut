//! The PNG decoder: a [`PngDecoder`] with hostile-input limits, implementing
//! [`gamut_core::DecodeImage`] for every pixel layout the file can fill losslessly.
//!
//! The pipeline follows the spec stage by stage: signature and chunk framing (§5), IHDR
//! validation (§11.2.1), chunk ordering (§5.6), IDAT concatenation and bounded zlib inflation
//! (§10), per-scanline defiltering (§9), and sub-byte unpacking (§7.2). Every allocation sized
//! from untrusted fields is guarded: dimensions are checked against the decoder's limits before
//! anything is allocated, and the inflater refuses to produce more bytes than the image geometry
//! implies, so a "zlib bomb" fails cleanly.
//!
//! The typed [`DecodeImage`] implementations perform **lossless widening only** — greyscale
//! replicates into RGB, an opaque alpha channel can be added, sub-byte greys scale exactly to
//! 8 bits (§13.12) — and refuse lossy requests (dropping alpha or transparency, narrowing 16-bit
//! samples) with [`Error::Unsupported`].

use gamut_core::{
    Bilevel, DecodeImage, Dimensions, Error, Gray8, Gray16, GrayAlpha8, GrayAlpha16, ImageBuf,
    Result, Rgb8, Rgb16, Rgba8, Rgba16,
};

use crate::chunk::ChunkReader;
use crate::color::ColorType;
use crate::filter::{self, FilterType};
use crate::ihdr::{self, Ihdr};
use crate::inflate;
use crate::pack;

/// Default cap on the decoded sample buffer: 64 MiB, a 4096×4096 RGBA8 image.
const DEFAULT_MAX_IMAGE_BYTES: usize = 64 << 20;
/// Default cumulative cap on inflated metadata (iCCP/zTXt/iTXt) payloads: 16 MiB.
const DEFAULT_MAX_METADATA_BYTES: usize = 16 << 20;
/// The spec's own dimension bound (§11.2.1): width and height are 1 ..= 2³¹ − 1.
const SPEC_MAX_DIMENSION: u32 = i32::MAX as u32;

/// The one refusal message for typed decodes the layout cannot hold losslessly.
const LOSSY: Error = Error::Unsupported(
    "PNG: this pixel layout cannot hold the image losslessly; use PngDecoder::decode",
);

/// A reusable PNG decoder with hostile-input limits.
///
/// The defaults accept anything the spec allows dimensionally but cap the decoded sample buffer
/// at 64 MiB — the byte budget, not the dimension caps, is the real safety guard (it bounds both
/// the inflated scanline stream and the sample buffer, so peak memory stays within roughly twice
/// the budget plus the input). Tighten the dimension caps when the application knows its domain.
#[derive(Debug, Clone)]
pub struct PngDecoder {
    max_width: u32,
    max_height: u32,
    max_image_bytes: usize,
    max_metadata_bytes: usize,
}

impl Default for PngDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl PngDecoder {
    /// Creates a decoder with the default limits (spec-maximum dimensions, 64 MiB of decoded
    /// samples, 16 MiB of inflated metadata).
    #[must_use]
    pub fn new() -> Self {
        Self {
            max_width: SPEC_MAX_DIMENSION,
            max_height: SPEC_MAX_DIMENSION,
            max_image_bytes: DEFAULT_MAX_IMAGE_BYTES,
            max_metadata_bytes: DEFAULT_MAX_METADATA_BYTES,
        }
    }

    /// Caps the accepted image dimensions; a wider or taller image is refused before any
    /// allocation.
    #[must_use]
    pub fn with_max_dimensions(mut self, width: u32, height: u32) -> Self {
        self.max_width = width;
        self.max_height = height;
        self
    }

    /// Caps the decoded sample buffer in bytes (default 64 MiB).
    #[must_use]
    pub fn with_max_image_bytes(mut self, bytes: usize) -> Self {
        self.max_image_bytes = bytes;
        self
    }

    /// Caps the *cumulative* inflated size of compressed metadata payloads — iCCP, zTXt, and
    /// compressed iTXt together (default 16 MiB). Payloads past the budget are skipped, not
    /// errors; the typed [`DecodeImage`] path never inflates metadata at all.
    #[must_use]
    pub fn with_max_metadata_bytes(mut self, bytes: usize) -> Self {
        self.max_metadata_bytes = bytes;
        self
    }
}

/// A tRNS colour key for greyscale or truecolour images, in the file's **native** (unscaled)
/// sample range (§11.3.1.1). Pixels equal to the key are fully transparent, all others opaque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransparencyKey {
    /// The transparent grey level of a greyscale image.
    Gray(u16),
    /// The transparent RGB colour of a truecolour image.
    Rgb(u16, u16, u16),
}

/// The image-bearing chunks collected from one pass over the chunk stream.
struct Parsed<'a> {
    header: Ihdr,
    plte: Option<&'a [u8]>,
    trns: Option<&'a [u8]>,
    /// All IDAT payloads, concatenated (§5.6 requires them consecutive).
    idat: Vec<u8>,
}

/// Decoded samples in the file's native value range: one byte per sample for depths ≤ 8
/// (sub-byte depths unpacked but **unscaled**), native-endian `u16` for depth 16.
enum NativeSamples {
    B8(Vec<u8>),
    B16(Vec<u16>),
}

/// A fully decoded image in its native layout, before presentation as a pixel type.
struct NativeImage {
    header: Ihdr,
    samples: NativeSamples,
    trns_key: Option<TransparencyKey>,
}

impl PngDecoder {
    /// Walks the chunk stream, enforcing criticality, CRC, and ordering rules (§5.4–§5.6).
    fn parse_stream<'a>(&self, data: &'a [u8]) -> Result<Parsed<'a>> {
        let mut reader = ChunkReader::new(data)?;
        let first = reader
            .next_chunk()?
            .ok_or(Error::InvalidInput("PNG: missing IHDR"))?;
        if first.chunk_type != *b"IHDR" {
            return Err(Error::InvalidInput("PNG: first chunk must be IHDR"));
        }
        if !first.crc_ok {
            return Err(Error::InvalidInput("PNG: critical chunk CRC mismatch"));
        }
        let header = ihdr::parse(first.data)?;

        let mut plte: Option<&[u8]> = None;
        let mut trns: Option<&[u8]> = None;
        let mut idat = Vec::new();
        let mut seen_idat = false;
        let mut idat_done = false;
        let mut seen_iend = false;
        while let Some(chunk) = reader.next_chunk()? {
            if seen_idat && chunk.chunk_type != *b"IDAT" {
                idat_done = true;
            }
            match &chunk.chunk_type {
                b"IHDR" => return Err(Error::InvalidInput("PNG: duplicate IHDR")),
                b"IEND" => {
                    if !chunk.crc_ok {
                        return Err(Error::InvalidInput("PNG: critical chunk CRC mismatch"));
                    }
                    if !chunk.data.is_empty() {
                        return Err(Error::InvalidInput("PNG: IEND payload must be empty"));
                    }
                    seen_iend = true;
                    // Everything after IEND is not part of the PNG datastream; trailing bytes
                    // are ignored, as §13.2 asks decoders to be liberal about.
                    break;
                }
                b"IDAT" => {
                    if !chunk.crc_ok {
                        return Err(Error::InvalidInput("PNG: critical chunk CRC mismatch"));
                    }
                    if idat_done {
                        return Err(Error::InvalidInput("PNG: IDAT chunks must be consecutive"));
                    }
                    seen_idat = true;
                    idat.extend_from_slice(chunk.data);
                }
                b"PLTE" => {
                    if !chunk.crc_ok {
                        return Err(Error::InvalidInput("PNG: critical chunk CRC mismatch"));
                    }
                    if plte.is_some() {
                        return Err(Error::InvalidInput("PNG: duplicate PLTE"));
                    }
                    if seen_idat {
                        return Err(Error::InvalidInput("PNG: PLTE must precede IDAT"));
                    }
                    if trns.is_some() {
                        return Err(Error::InvalidInput("PNG: PLTE must precede tRNS"));
                    }
                    plte = Some(chunk.data);
                }
                b"tRNS" => {
                    if seen_idat {
                        return Err(Error::InvalidInput("PNG: tRNS must precede IDAT"));
                    }
                    // tRNS is ancillary: a CRC mismatch skips the chunk (§13.1); duplicates keep
                    // the first occurrence.
                    if chunk.crc_ok && trns.is_none() {
                        trns = Some(chunk.data);
                    }
                }
                _ if chunk.is_ancillary() => {
                    // Unknown or metadata-bearing ancillary chunks do not affect the pixels;
                    // they are collected (when requested) by the rich decode path.
                }
                _ => {
                    // §5.4/§13.2: a chunk that is critical but unknown means the image cannot
                    // be correctly rendered.
                    return Err(Error::Unsupported("PNG: unknown critical chunk"));
                }
            }
        }
        if !seen_iend {
            return Err(Error::InvalidInput("PNG: missing IEND"));
        }
        if !seen_idat {
            return Err(Error::InvalidInput("PNG: missing IDAT"));
        }
        Ok(Parsed {
            header,
            plte,
            trns,
            idat,
        })
    }

    /// Enforces the dimension and byte-budget limits (all math checked) and returns the exact
    /// byte length of the filtered scanline stream the IDAT data must inflate to.
    fn check_limits(&self, header: &Ihdr) -> Result<usize> {
        if header.width > self.max_width || header.height > self.max_height {
            return Err(Error::Unsupported("PNG: image exceeds the dimension limit"));
        }
        let (width, height) = (header.width as usize, header.height as usize);
        // Budget the *decoded* representation: one byte per sample below depth 16 (sub-byte
        // depths are unpacked), two above.
        let bytes_per_sample = if header.bit_depth == 16 { 2 } else { 1 };
        let native_bytes = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(header.color.channels()))
            .and_then(|samples| samples.checked_mul(bytes_per_sample))
            .ok_or(Error::InvalidInput("PNG: image dimensions overflow"))?;
        if native_bytes > self.max_image_bytes {
            return Err(Error::Unsupported("PNG: image exceeds the size limit"));
        }
        expected_stream_len(header).ok_or(Error::InvalidInput("PNG: image dimensions overflow"))
    }

    /// Runs the shared pipeline: parse → validate PLTE/tRNS → inflate → defilter → unpack.
    fn decode_native(&self, data: &[u8]) -> Result<NativeImage> {
        let parsed = self.parse_stream(data)?;
        let header = parsed.header;
        if header.interlaced {
            return Err(Error::Unsupported("PNG: Adam7 decode lands in a later phase"));
        }
        if header.color == ColorType::Indexed {
            return Err(Error::Unsupported("PNG: indexed decode lands in a later phase"));
        }
        let trns_key = validate_plte_and_trns(&header, parsed.plte, parsed.trns)?;

        let expected = self.check_limits(&header)?;
        let stream = inflate::inflate_zlib(&parsed.idat, expected)?;
        if stream.len() != expected {
            return Err(Error::InvalidInput("PNG: IDAT is shorter than the image"));
        }

        let (width, height) = (header.width as usize, header.height as usize);
        let row_bytes = packed_row_bytes(header.width, header.bits_per_pixel())
            .ok_or(Error::InvalidInput("PNG: image dimensions overflow"))?;
        let bpp = filter_stride(&header);
        let packed = unfilter_stream(&stream, row_bytes, height, bpp)?;

        let samples = match header.bit_depth {
            16 => NativeSamples::B16(
                packed
                    .chunks_exact(2)
                    .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                    .collect(),
            ),
            8 => NativeSamples::B8(packed),
            depth => NativeSamples::B8(pack::unpack_scanlines(&packed, width, height, depth)),
        };
        Ok(NativeImage {
            header,
            samples,
            trns_key,
        })
    }
}

/// Validates PLTE presence/shape and tRNS shape against the colour type (§11.2.2, §11.3.1.1),
/// returning the parsed colour key for greyscale/truecolour images.
fn validate_plte_and_trns(
    header: &Ihdr,
    plte: Option<&[u8]>,
    trns: Option<&[u8]>,
) -> Result<Option<TransparencyKey>> {
    if let Some(plte) = plte {
        match header.color {
            ColorType::Grayscale | ColorType::GrayscaleAlpha => {
                return Err(Error::InvalidInput(
                    "PNG: PLTE is forbidden for greyscale colour types",
                ));
            }
            // For truecolour it is a suggested quantisation palette (§11.2.2): shape-checked,
            // then ignored.
            ColorType::Truecolor | ColorType::TruecolorAlpha | ColorType::Indexed => {
                let entries = plte.len() / 3;
                if plte.len() % 3 != 0 || !(1..=256).contains(&entries) {
                    return Err(Error::InvalidInput("PNG: malformed PLTE payload"));
                }
            }
        }
    }
    let Some(trns) = trns else { return Ok(None) };
    match header.color {
        ColorType::Grayscale => {
            let bytes: &[u8; 2] = trns
                .try_into()
                .map_err(|_| Error::InvalidInput("PNG: malformed tRNS payload"))?;
            let key = u16::from_be_bytes(*bytes) & depth_mask(header.bit_depth);
            Ok(Some(TransparencyKey::Gray(key)))
        }
        ColorType::Truecolor => {
            let bytes: &[u8; 6] = trns
                .try_into()
                .map_err(|_| Error::InvalidInput("PNG: malformed tRNS payload"))?;
            let mask = depth_mask(header.bit_depth);
            Ok(Some(TransparencyKey::Rgb(
                u16::from_be_bytes([bytes[0], bytes[1]]) & mask,
                u16::from_be_bytes([bytes[2], bytes[3]]) & mask,
                u16::from_be_bytes([bytes[4], bytes[5]]) & mask,
            )))
        }
        // Indexed tRNS folds into the palette (handled with palette decode).
        ColorType::Indexed => Ok(None),
        ColorType::GrayscaleAlpha | ColorType::TruecolorAlpha => Err(Error::InvalidInput(
            "PNG: tRNS is forbidden for colour types with alpha",
        )),
    }
}

/// The native-range mask for a colour key: §11.3.1.1 stores keys as 2-byte values but only the
/// low `bit_depth` bits are significant below depth 16.
fn depth_mask(bit_depth: u8) -> u16 {
    if bit_depth >= 16 {
        u16::MAX
    } else {
        (1 << bit_depth) - 1
    }
}

/// Bytes of one packed scanline: `ceil(width × bits_per_pixel / 8)` (§7.2), checked.
fn packed_row_bytes(width: u32, bits_per_pixel: usize) -> Option<usize> {
    (width as usize)
        .checked_mul(bits_per_pixel)
        .map(|bits| bits.div_ceil(8))
}

/// The filter's byte stride (§9.2): whole bytes per pixel, rounded up to at least one.
fn filter_stride(header: &Ihdr) -> usize {
    (header.bits_per_pixel() / 8).max(1)
}

/// The exact byte length of the filtered scanline stream: per scanline, one filter-type byte
/// plus the packed row (§7.2/§7.3). `None` on arithmetic overflow.
fn expected_stream_len(header: &Ihdr) -> Option<usize> {
    let row = packed_row_bytes(header.width, header.bits_per_pixel())?;
    (header.height as usize).checked_mul(row.checked_add(1)?)
}

/// Defilters a scanline stream (`height` rows of `1 + row_bytes` bytes) into packed raw rows
/// (§9.2–§9.4).
fn unfilter_stream(stream: &[u8], row_bytes: usize, height: usize, bpp: usize) -> Result<Vec<u8>> {
    debug_assert_eq!(stream.len(), height * (row_bytes + 1));
    let mut out = vec![0u8; row_bytes * height];
    let zero_row = vec![0u8; row_bytes];
    for y in 0..height {
        let src = &stream[y * (row_bytes + 1)..(y + 1) * (row_bytes + 1)];
        let filter = FilterType::from_code(src[0])
            .ok_or(Error::InvalidInput("PNG: undefined filter type"))?;
        let (done, rest) = out.split_at_mut(y * row_bytes);
        let prev = if y == 0 {
            zero_row.as_slice()
        } else {
            &done[(y - 1) * row_bytes..]
        };
        let cur = &mut rest[..row_bytes];
        cur.copy_from_slice(&src[1..]);
        filter::unfilter_row(filter, cur, prev, bpp);
    }
    Ok(out)
}

/// The exact §13.12 factor presenting a sub-byte grey sample at 8 bits: 255 / (2^depth − 1).
fn gray8_scale(bit_depth: u8) -> u8 {
    match bit_depth {
        1 => 255,
        2 => 85,
        4 => 17,
        _ => 1,
    }
}

/// The 8-bit samples of a native image, or the lossless-refusal error.
fn native8(samples: NativeSamples) -> Result<Vec<u8>> {
    match samples {
        NativeSamples::B8(v) => Ok(v),
        NativeSamples::B16(_) => Err(LOSSY),
    }
}

/// The 16-bit samples of a native image, or the lossless-refusal error.
fn native16(samples: NativeSamples) -> Result<Vec<u16>> {
    match samples {
        NativeSamples::B16(v) => Ok(v),
        NativeSamples::B8(_) => Err(LOSSY),
    }
}

/// `Dimensions` for a validated header (post-IHDR this cannot fail, but stays checked).
fn dims(header: &Ihdr) -> Result<Dimensions> {
    Dimensions::new(header.width, header.height)
}

// --- Typed presentation ------------------------------------------------------------------------
//
// Each impl accepts exactly the native layouts the pixel type holds losslessly (see the module
// docs) and funnels through `decode_native`. The typed path never inflates metadata chunks.

impl DecodeImage<Bilevel> for PngDecoder {
    /// Accepts 1-bit greyscale without transparency; samples are presented as 0/1.
    fn decode_image(&self, data: &[u8]) -> Result<ImageBuf<Bilevel>> {
        let native = self.decode_native(data)?;
        if native.header.color != ColorType::Grayscale
            || native.header.bit_depth != 1
            || native.trns_key.is_some()
        {
            return Err(LOSSY);
        }
        ImageBuf::new(native8(native.samples)?, dims(&native.header)?)
    }
}

impl DecodeImage<Gray8> for PngDecoder {
    /// Accepts 1/2/4/8-bit greyscale without transparency; sub-byte samples scale exactly to
    /// 8 bits (§13.12).
    fn decode_image(&self, data: &[u8]) -> Result<ImageBuf<Gray8>> {
        let native = self.decode_native(data)?;
        if native.header.color != ColorType::Grayscale || native.trns_key.is_some() {
            return Err(LOSSY);
        }
        let scale = gray8_scale(native.header.bit_depth);
        let mut samples = native8(native.samples)?;
        for value in &mut samples {
            *value *= scale;
        }
        ImageBuf::new(samples, dims(&native.header)?)
    }
}

impl DecodeImage<GrayAlpha8> for PngDecoder {
    /// Accepts 8-bit grey+alpha, and 1/2/4/8-bit greyscale (opaque, or keyed by tRNS).
    fn decode_image(&self, data: &[u8]) -> Result<ImageBuf<GrayAlpha8>> {
        let native = self.decode_native(data)?;
        let out = match native.header.color {
            ColorType::GrayscaleAlpha if native.header.bit_depth == 8 => {
                native8(native.samples)?
            }
            ColorType::Grayscale if native.header.bit_depth <= 8 => {
                let key = gray_key(native.trns_key);
                let scale = gray8_scale(native.header.bit_depth);
                let grays = native8(native.samples)?;
                let mut out = Vec::with_capacity(grays.len() * 2);
                for &gray in &grays {
                    out.push(gray * scale);
                    out.push(alpha8(key != Some(u16::from(gray))));
                }
                out
            }
            _ => return Err(LOSSY),
        };
        ImageBuf::new(out, dims(&native.header)?)
    }
}

impl DecodeImage<Rgb8> for PngDecoder {
    /// Accepts 8-bit RGB, and 1/2/4/8-bit greyscale, both without transparency.
    fn decode_image(&self, data: &[u8]) -> Result<ImageBuf<Rgb8>> {
        let native = self.decode_native(data)?;
        if native.trns_key.is_some() {
            return Err(LOSSY);
        }
        let out = match native.header.color {
            ColorType::Truecolor if native.header.bit_depth == 8 => native8(native.samples)?,
            ColorType::Grayscale if native.header.bit_depth <= 8 => {
                let scale = gray8_scale(native.header.bit_depth);
                native8(native.samples)?
                    .iter()
                    .flat_map(|&gray| [gray * scale; 3])
                    .collect()
            }
            _ => return Err(LOSSY),
        };
        ImageBuf::new(out, dims(&native.header)?)
    }
}

impl DecodeImage<Rgba8> for PngDecoder {
    /// Accepts 8-bit RGBA and, by lossless widening, 8-bit RGB / grey+alpha and 1/2/4/8-bit
    /// greyscale, honouring a tRNS colour key.
    fn decode_image(&self, data: &[u8]) -> Result<ImageBuf<Rgba8>> {
        let native = self.decode_native(data)?;
        let out = match native.header.color {
            ColorType::TruecolorAlpha if native.header.bit_depth == 8 => {
                native8(native.samples)?
            }
            ColorType::Truecolor if native.header.bit_depth == 8 => {
                let key = native.trns_key;
                native8(native.samples)?
                    .chunks_exact(3)
                    .flat_map(|px| {
                        let transparent = key
                            == Some(TransparencyKey::Rgb(
                                u16::from(px[0]),
                                u16::from(px[1]),
                                u16::from(px[2]),
                            ));
                        [px[0], px[1], px[2], alpha8(!transparent)]
                    })
                    .collect()
            }
            ColorType::GrayscaleAlpha if native.header.bit_depth == 8 => {
                native8(native.samples)?
                    .chunks_exact(2)
                    .flat_map(|px| [px[0], px[0], px[0], px[1]])
                    .collect()
            }
            ColorType::Grayscale if native.header.bit_depth <= 8 => {
                let key = gray_key(native.trns_key);
                let scale = gray8_scale(native.header.bit_depth);
                native8(native.samples)?
                    .iter()
                    .flat_map(|&gray| {
                        let alpha = alpha8(key != Some(u16::from(gray)));
                        [gray * scale, gray * scale, gray * scale, alpha]
                    })
                    .collect()
            }
            _ => return Err(LOSSY),
        };
        ImageBuf::new(out, dims(&native.header)?)
    }
}

impl DecodeImage<Gray16> for PngDecoder {
    /// Accepts 16-bit greyscale without transparency.
    fn decode_image(&self, data: &[u8]) -> Result<ImageBuf<Gray16>> {
        let native = self.decode_native(data)?;
        if native.header.color != ColorType::Grayscale || native.trns_key.is_some() {
            return Err(LOSSY);
        }
        ImageBuf::new(native16(native.samples)?, dims(&native.header)?)
    }
}

impl DecodeImage<GrayAlpha16> for PngDecoder {
    /// Accepts 16-bit grey+alpha, and 16-bit greyscale (opaque, or keyed by tRNS).
    fn decode_image(&self, data: &[u8]) -> Result<ImageBuf<GrayAlpha16>> {
        let native = self.decode_native(data)?;
        let out = match native.header.color {
            ColorType::GrayscaleAlpha if native.header.bit_depth == 16 => {
                native16(native.samples)?
            }
            ColorType::Grayscale if native.header.bit_depth == 16 => {
                let key = gray_key(native.trns_key);
                native16(native.samples)?
                    .iter()
                    .flat_map(|&gray| [gray, alpha16(key != Some(gray))])
                    .collect()
            }
            _ => return Err(LOSSY),
        };
        ImageBuf::new(out, dims(&native.header)?)
    }
}

impl DecodeImage<Rgb16> for PngDecoder {
    /// Accepts 16-bit RGB and 16-bit greyscale, both without transparency.
    fn decode_image(&self, data: &[u8]) -> Result<ImageBuf<Rgb16>> {
        let native = self.decode_native(data)?;
        if native.trns_key.is_some() {
            return Err(LOSSY);
        }
        let out = match native.header.color {
            ColorType::Truecolor if native.header.bit_depth == 16 => native16(native.samples)?,
            ColorType::Grayscale if native.header.bit_depth == 16 => native16(native.samples)?
                .iter()
                .flat_map(|&gray| [gray; 3])
                .collect(),
            _ => return Err(LOSSY),
        };
        ImageBuf::new(out, dims(&native.header)?)
    }
}

impl DecodeImage<Rgba16> for PngDecoder {
    /// Accepts every 16-bit layout: RGBA natively and, by lossless widening, RGB / grey+alpha /
    /// greyscale, honouring a tRNS colour key.
    fn decode_image(&self, data: &[u8]) -> Result<ImageBuf<Rgba16>> {
        let native = self.decode_native(data)?;
        let out = match native.header.color {
            ColorType::TruecolorAlpha if native.header.bit_depth == 16 => {
                native16(native.samples)?
            }
            ColorType::Truecolor if native.header.bit_depth == 16 => {
                let key = native.trns_key;
                native16(native.samples)?
                    .chunks_exact(3)
                    .flat_map(|px| {
                        let transparent =
                            key == Some(TransparencyKey::Rgb(px[0], px[1], px[2]));
                        [px[0], px[1], px[2], alpha16(!transparent)]
                    })
                    .collect()
            }
            ColorType::GrayscaleAlpha if native.header.bit_depth == 16 => {
                native16(native.samples)?
                    .chunks_exact(2)
                    .flat_map(|px| [px[0], px[0], px[0], px[1]])
                    .collect()
            }
            ColorType::Grayscale if native.header.bit_depth == 16 => {
                let key = gray_key(native.trns_key);
                native16(native.samples)?
                    .iter()
                    .flat_map(|&gray| [gray, gray, gray, alpha16(key != Some(gray))])
                    .collect()
            }
            _ => return Err(LOSSY),
        };
        ImageBuf::new(out, dims(&native.header)?)
    }
}

/// The grey colour key, if one applies.
fn gray_key(key: Option<TransparencyKey>) -> Option<u16> {
    match key {
        Some(TransparencyKey::Gray(gray)) => Some(gray),
        _ => None,
    }
}

/// 8-bit alpha: fully opaque or fully transparent.
fn alpha8(opaque: bool) -> u8 {
    if opaque { 255 } else { 0 }
}

/// 16-bit alpha: fully opaque or fully transparent.
fn alpha16(opaque: bool) -> u16 {
    if opaque { u16::MAX } else { 0 }
}

#[cfg(test)]
mod tests {
    use gamut_core::{EncodeImage, ImageRef};

    use crate::PngEncoder;

    use super::*;

    fn rgb_png(w: u32, h: u32) -> (Vec<u8>, Vec<u8>) {
        let src: Vec<u8> = (0..(w * h * 3) as usize)
            .map(|i| (i.wrapping_mul(41) ^ (i >> 3)) as u8)
            .collect();
        let mut png = Vec::new();
        PngEncoder::new()
            .encode_image(
                ImageRef::<Rgb8>::new(&src, Dimensions::new(w, h).unwrap()).unwrap(),
                &mut png,
            )
            .unwrap();
        (png, src)
    }

    #[test]
    fn decodes_own_encoder_output() {
        for (w, h) in [(1, 1), (3, 2), (17, 13), (40, 30)] {
            let (png, src) = rgb_png(w, h);
            let img: ImageBuf<Rgb8> = PngDecoder::new().decode_image(&png).unwrap();
            assert_eq!(img.dimensions(), Dimensions::new(w, h).unwrap());
            assert_eq!(img.as_samples(), src, "{w}x{h}");
        }
    }

    #[test]
    fn widens_gray_to_rgb_and_rgba() {
        let src: Vec<u8> = (0..20u8).collect();
        let mut png = Vec::new();
        PngEncoder::new()
            .encode_image(
                ImageRef::<Gray8>::new(&src, Dimensions::new(5, 4).unwrap()).unwrap(),
                &mut png,
            )
            .unwrap();
        let rgb: ImageBuf<Rgb8> = PngDecoder::new().decode_image(&png).unwrap();
        let expected_rgb: Vec<u8> = src.iter().flat_map(|&g| [g; 3]).collect();
        assert_eq!(rgb.as_samples(), expected_rgb);
        let rgba: ImageBuf<Rgba8> = PngDecoder::new().decode_image(&png).unwrap();
        let expected_rgba: Vec<u8> = src.iter().flat_map(|&g| [g, g, g, 255]).collect();
        assert_eq!(rgba.as_samples(), expected_rgba);
    }

    #[test]
    fn refuses_lossy_requests() {
        let src = vec![0u16; 4];
        let mut png = Vec::new();
        PngEncoder::new()
            .encode_image(
                ImageRef::<Gray16>::new(&src, Dimensions::new(2, 2).unwrap()).unwrap(),
                &mut png,
            )
            .unwrap();
        // A 16-bit file cannot decode into 8-bit layouts.
        assert!(DecodeImage::<Gray8>::decode_image(&PngDecoder::new(), &png).is_err());
        assert!(DecodeImage::<Rgba8>::decode_image(&PngDecoder::new(), &png).is_err());
        // But it widens losslessly to 16-bit RGB.
        let rgb: ImageBuf<Rgb16> = PngDecoder::new().decode_image(&png).unwrap();
        assert_eq!(rgb.as_samples(), vec![0u16; 12]);
    }

    #[test]
    fn dimension_limit_is_exact() {
        let (png, _) = rgb_png(17, 13);
        let decoder = PngDecoder::new().with_max_dimensions(17, 13);
        assert!(DecodeImage::<Rgb8>::decode_image(&decoder, &png).is_ok());
        let narrow = PngDecoder::new().with_max_dimensions(16, 13);
        assert!(DecodeImage::<Rgb8>::decode_image(&narrow, &png).is_err());
        let short = PngDecoder::new().with_max_dimensions(17, 12);
        assert!(DecodeImage::<Rgb8>::decode_image(&short, &png).is_err());
    }

    #[test]
    fn byte_budget_is_exact() {
        let (png, _) = rgb_png(8, 8);
        let exact = PngDecoder::new().with_max_image_bytes(8 * 8 * 3);
        assert!(DecodeImage::<Rgb8>::decode_image(&exact, &png).is_ok());
        let tight = PngDecoder::new().with_max_image_bytes(8 * 8 * 3 - 1);
        assert!(matches!(
            DecodeImage::<Rgb8>::decode_image(&tight, &png),
            Err(Error::Unsupported(_))
        ));
    }
}
