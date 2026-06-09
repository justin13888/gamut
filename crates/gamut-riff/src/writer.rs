//! Writing the RIFF/WebP chunk structure: a 12-byte `RIFF`/size/`WEBP` header followed by padded
//! chunks (RFC 9649 §2.3-§2.4).

use crate::chunk::ChunkHeader;
use crate::fourcc::FourCc;

/// Builder for a RIFF/WebP byte stream.
///
/// Begin with [`RiffWriter::new`] (which emits the `RIFF` magic, a placeholder file size, and the
/// `WEBP` form), append chunks with [`RiffWriter::write_chunk`], then call [`RiffWriter::finish`] to
/// back-patch the file size and obtain the finished bytes.
#[derive(Debug, Clone)]
pub struct RiffWriter {
    /// Accumulated output. Bytes `4..8` hold the file-size placeholder patched in `finish`.
    buf: Vec<u8>,
}

impl RiffWriter {
    /// Creates a writer for a WebP file, emitting the 12-byte `RIFF` / size / `WEBP` header with the
    /// size left as a placeholder to be patched in [`RiffWriter::finish`].
    #[must_use]
    pub fn new() -> Self {
        let mut buf = Vec::with_capacity(12);
        buf.extend_from_slice(FourCc::RIFF.as_bytes());
        buf.extend_from_slice(&[0, 0, 0, 0]); // file-size placeholder, patched in finish()
        buf.extend_from_slice(FourCc::WEBP.as_bytes());
        Self { buf }
    }

    /// Appends one chunk: its FourCC, the `uint32` little-endian payload size, the payload, and a
    /// single zero pad byte when the payload length is odd (RFC 9649 §2.3).
    ///
    /// A payload longer than `u32::MAX` cannot occur for a still image; the size field saturates
    /// rather than panicking if one is somehow passed.
    pub fn write_chunk(&mut self, fourcc: FourCc, payload: &[u8]) {
        let size = u32::try_from(payload.len()).unwrap_or(u32::MAX);
        self.buf.extend_from_slice(fourcc.as_bytes());
        self.buf.extend_from_slice(&size.to_le_bytes());
        self.buf.extend_from_slice(payload);
        if ChunkHeader::padding(size) == 1 {
            self.buf.push(0);
        }
    }

    /// Back-patches the RIFF file-size field (the byte count following the 8-byte `RIFF`+size
    /// prefix: the `WEBP` form plus all chunks) and returns the finished byte stream.
    #[must_use]
    pub fn finish(mut self) -> Vec<u8> {
        let file_size = u32::try_from(self.buf.len() - 8).unwrap_or(u32::MAX);
        self.buf[4..8].copy_from_slice(&file_size.to_le_bytes());
        self.buf
    }
}

impl Default for RiffWriter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_is_riff_placeholder_webp() {
        let w = RiffWriter::new();
        assert_eq!(&w.buf[0..4], b"RIFF");
        assert_eq!(&w.buf[4..8], &[0, 0, 0, 0]);
        assert_eq!(&w.buf[8..12], b"WEBP");
    }

    #[test]
    fn even_payload_has_no_pad_byte() {
        let mut w = RiffWriter::new();
        w.write_chunk(FourCc::VP8L, &[1, 2, 3, 4]);
        let out = w.finish();
        // 12 header + 8 chunk header + 4 payload, no pad.
        assert_eq!(out.len(), 12 + 8 + 4);
        assert_eq!(&out[12..16], b"VP8L");
        assert_eq!(&out[16..20], &4u32.to_le_bytes());
    }

    #[test]
    fn odd_payload_gets_one_zero_pad_byte() {
        let mut w = RiffWriter::new();
        w.write_chunk(FourCc::VP8, &[9, 8, 7]);
        let out = w.finish();
        // 12 header + 8 chunk header + 3 payload + 1 pad.
        assert_eq!(out.len(), 12 + 8 + 3 + 1);
        assert_eq!(*out.last().unwrap(), 0, "odd payload must be zero-padded");
        // Size field records the *unpadded* length.
        assert_eq!(&out[16..20], &3u32.to_le_bytes());
    }

    #[test]
    fn finish_patches_file_size() {
        let mut w = RiffWriter::new();
        w.write_chunk(FourCc::VP8L, &[0; 6]);
        let out = w.finish();
        let file_size = u32::from_le_bytes([out[4], out[5], out[6], out[7]]) as usize;
        assert_eq!(
            file_size,
            out.len() - 8,
            "file size counts everything after the size field"
        );
        assert_eq!(file_size & 1, 0, "WebP file size is always even");
    }

    #[test]
    fn default_matches_new() {
        assert_eq!(RiffWriter::default().finish(), RiffWriter::new().finish());
    }
}
