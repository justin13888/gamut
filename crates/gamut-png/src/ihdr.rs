//! The IHDR (image header) chunk — the first chunk after the signature (PNG spec §11.2.1).

use gamut_core::{Error, Result};

use crate::chunk;
use crate::color::ColorType;

/// Appends an IHDR chunk describing a non-interlaced image using the standard DEFLATE compression
/// and adaptive filtering methods (the only ones PNG defines).
pub(crate) fn write(out: &mut Vec<u8>, width: u32, height: u32, bit_depth: u8, color: ColorType) {
    let mut data = [0u8; 13];
    data[0..4].copy_from_slice(&width.to_be_bytes());
    data[4..8].copy_from_slice(&height.to_be_bytes());
    data[8] = bit_depth;
    data[9] = color.code();
    data[10] = 0; // compression method: 0 = deflate (the only defined value)
    data[11] = 0; // filter method: 0 = adaptive per-scanline (the only defined value)
    data[12] = 0; // interlace method: 0 = none
    chunk::write_chunk(out, *b"IHDR", &data);
}

/// A parsed and validated image header (PNG spec §11.2.1).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Ihdr {
    /// Image width in pixels (1 ..= 2³¹ − 1).
    pub width: u32,
    /// Image height in pixels (1 ..= 2³¹ − 1).
    pub height: u32,
    /// Bits per sample (or per palette index): 1, 2, 4, 8, or 16, as Table 12 allows.
    pub bit_depth: u8,
    /// The colour type.
    pub color: ColorType,
    /// Whether the image is Adam7-interlaced (interlace method 1).
    pub interlaced: bool,
}

impl Ihdr {
    /// Bits per pixel: channels × bit depth (never overflows: ≤ 4 × 16).
    pub(crate) fn bits_per_pixel(&self) -> usize {
        self.color.channels() * self.bit_depth as usize
    }
}

/// Parses and validates a 13-byte IHDR payload (PNG spec §11.2.1).
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] for a wrong payload length, a zero or ≥ 2³¹ dimension, an
/// undefined colour type, a bit depth Table 12 forbids, or a non-zero compression, filter, or
/// unknown interlace method code.
pub(crate) fn parse(data: &[u8]) -> Result<Ihdr> {
    let data: &[u8; 13] = data
        .try_into()
        .map_err(|_| Error::InvalidInput("PNG: IHDR payload must be 13 bytes"))?;
    let width = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    let height = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    if width == 0 || height == 0 {
        return Err(Error::InvalidInput("PNG: zero image dimension"));
    }
    if width >= 1 << 31 || height >= 1 << 31 {
        return Err(Error::InvalidInput("PNG: dimension exceeds 2^31 - 1"));
    }
    let bit_depth = data[8];
    let color =
        ColorType::from_code(data[9]).ok_or(Error::InvalidInput("PNG: undefined colour type"))?;
    if !color.allows_bit_depth(bit_depth) {
        return Err(Error::InvalidInput(
            "PNG: bit depth not allowed for the colour type",
        ));
    }
    if data[10] != 0 {
        return Err(Error::InvalidInput("PNG: unknown compression method"));
    }
    if data[11] != 0 {
        return Err(Error::InvalidInput("PNG: unknown filter method"));
    }
    let interlaced = match data[12] {
        0 => false,
        1 => true,
        _ => return Err(Error::InvalidInput("PNG: unknown interlace method")),
    };
    Ok(Ihdr {
        width,
        height,
        bit_depth,
        color,
        interlaced,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ihdr_encodes_dimensions_and_type() {
        let mut out = Vec::new();
        write(&mut out, 0x0102_0304, 0x0506_0708, 8, ColorType::Truecolor);
        // 4-byte length + "IHDR" + 13 data bytes + 4-byte CRC.
        assert_eq!(out.len(), 4 + 4 + 13 + 4);
        assert_eq!(out[..4], 13u32.to_be_bytes());
        assert_eq!(&out[4..8], b"IHDR");
        assert_eq!(&out[8..12], &[1, 2, 3, 4]); // width, big-endian
        assert_eq!(&out[12..16], &[5, 6, 7, 8]); // height, big-endian
        assert_eq!(out[16], 8); // bit depth
        assert_eq!(out[17], 2); // colour type (truecolour)
        assert_eq!(&out[18..21], &[0, 0, 0]); // compression, filter, interlace
    }

    /// A valid 13-byte IHDR payload to mutate in the rejection tests.
    fn payload(width: u32, height: u32, depth: u8, color: u8, interlace: u8) -> [u8; 13] {
        let mut data = [0u8; 13];
        data[0..4].copy_from_slice(&width.to_be_bytes());
        data[4..8].copy_from_slice(&height.to_be_bytes());
        data[8] = depth;
        data[9] = color;
        data[12] = interlace;
        data
    }

    #[test]
    fn parse_reads_back_what_write_produces() {
        let mut out = Vec::new();
        write(&mut out, 640, 480, 16, ColorType::GrayscaleAlpha);
        let parsed = parse(&out[8..21]).unwrap();
        assert_eq!((parsed.width, parsed.height), (640, 480));
        assert_eq!(parsed.bit_depth, 16);
        assert_eq!(parsed.color, ColorType::GrayscaleAlpha);
        assert!(!parsed.interlaced);
        assert_eq!(parsed.bits_per_pixel(), 32);
    }

    #[test]
    fn parse_accepts_adam7() {
        let parsed = parse(&payload(3, 2, 8, 2, 1)).unwrap();
        assert!(parsed.interlaced);
    }

    #[test]
    fn parse_rejects_invalid_headers() {
        assert!(parse(&[0; 12]).is_err()); // wrong length
        assert!(parse(&[0; 14]).is_err());
        assert!(parse(&payload(0, 1, 8, 2, 0)).is_err()); // zero width
        assert!(parse(&payload(1, 0, 8, 2, 0)).is_err()); // zero height
        assert!(parse(&payload(1 << 31, 1, 8, 2, 0)).is_err()); // width bit 31
        assert!(parse(&payload(1, 1 << 31, 8, 2, 0)).is_err()); // height bit 31
        assert!(parse(&payload(1, 1, 8, 1, 0)).is_err()); // undefined colour type
        assert!(parse(&payload(1, 1, 4, 2, 0)).is_err()); // depth 4 forbidden for RGB
        assert!(parse(&payload(1, 1, 16, 3, 0)).is_err()); // depth 16 forbidden for indexed
        assert!(parse(&payload(1, 1, 8, 2, 2)).is_err()); // unknown interlace method
        let mut bad_compression = payload(1, 1, 8, 2, 0);
        bad_compression[10] = 1;
        assert!(parse(&bad_compression).is_err());
        let mut bad_filter = payload(1, 1, 8, 2, 0);
        bad_filter[11] = 1;
        assert!(parse(&bad_filter).is_err());
    }
}
