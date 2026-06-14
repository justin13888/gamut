//! The IHDR (image header) chunk — the first chunk after the signature (PNG spec §11.2.1).

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
}
