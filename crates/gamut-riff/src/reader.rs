//! Reading the RIFF/WebP chunk structure: validate the 12-byte WebP file header, then iterate the
//! contained chunks (RFC 9649 §2.3-§2.4).

use gamut_core::{Error, Result};

use crate::chunk::{CHUNK_HEADER_LEN, Chunk, ChunkHeader};
use crate::fourcc::FourCc;

/// Iterator over the top-level chunks of a WebP file's `RIFF`/`WEBP` payload.
///
/// Build one with [`RiffReader::new`], which validates the 12-byte WebP file header (`RIFF` + file
/// size + `WEBP`) and bounds the chunk region to the declared file size. Each [`Iterator::next`]
/// yields the next [`Chunk`], or an [`Error::InvalidInput`] (after which iteration ends) when the
/// stream is truncated or a chunk's size runs past the available data.
#[derive(Debug, Clone)]
pub struct RiffReader<'a> {
    /// Remaining bytes, positioned at the next chunk header. Emptied once exhausted or on error.
    rest: &'a [u8],
}

impl<'a> RiffReader<'a> {
    /// Parses the 12-byte WebP file header and returns a reader positioned at the first chunk.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `data` is shorter than the 12-byte header, does not begin
    /// with the `RIFF` magic, has a form other than `WEBP`, or declares a file size that runs past
    /// the end of `data`.
    pub fn new(data: &'a [u8]) -> Result<Self> {
        // 12-byte header: 'RIFF' (4) + file size (4, little-endian) + form (4).
        if data.len() < 12 {
            return Err(Error::InvalidInput(
                "RIFF: shorter than 12-byte file header",
            ));
        }
        if &data[0..4] != FourCc::RIFF.as_bytes() {
            return Err(Error::InvalidInput("RIFF: missing RIFF magic"));
        }
        if &data[8..12] != FourCc::WEBP.as_bytes() {
            return Err(Error::InvalidInput("RIFF: form is not WEBP"));
        }
        // file_size counts everything after the size field: the 'WEBP' form (4) plus the chunks. It
        // must cover at least the form and must not claim more bytes than are present.
        let file_size = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
        if file_size < 4 || file_size > data.len() - 8 {
            return Err(Error::InvalidInput("RIFF: declared file size out of range"));
        }
        // Chunks occupy bytes 12..(8 + file_size); ignore any trailing data past that point.
        Ok(Self {
            rest: &data[12..8 + file_size],
        })
    }
}

impl<'a> Iterator for RiffReader<'a> {
    type Item = Result<Chunk<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.rest.is_empty() {
            return None;
        }
        if self.rest.len() < CHUNK_HEADER_LEN {
            self.rest = &[];
            return Some(Err(Error::InvalidInput("RIFF: truncated chunk header")));
        }
        let fourcc = FourCc([self.rest[0], self.rest[1], self.rest[2], self.rest[3]]);
        let size =
            u32::from_le_bytes([self.rest[4], self.rest[5], self.rest[6], self.rest[7]]) as usize;
        let avail = self.rest.len() - CHUNK_HEADER_LEN;
        if size > avail {
            self.rest = &[];
            return Some(Err(Error::InvalidInput(
                "RIFF: chunk size exceeds remaining data",
            )));
        }
        let payload = &self.rest[CHUNK_HEADER_LEN..CHUNK_HEADER_LEN + size];
        // Advance past header + payload + the RIFF pad byte. A non-conforming final chunk may omit
        // the pad byte; `get` clamps so the reader simply ends instead of erroring on it.
        let consumed = CHUNK_HEADER_LEN + size + ChunkHeader::padding(size as u32);
        self.rest = self.rest.get(consumed..).unwrap_or(&[]);
        Some(Ok(Chunk { fourcc, payload }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::RiffWriter;

    /// Builds a WebP file with the given chunks via the writer (the round-trip counterpart).
    fn build(chunks: &[(FourCc, &[u8])]) -> Vec<u8> {
        let mut w = RiffWriter::new();
        for (fourcc, payload) in chunks {
            w.write_chunk(*fourcc, payload);
        }
        w.finish()
    }

    #[test]
    fn roundtrips_multiple_chunks() {
        let file = build(&[
            (FourCc::VP8X, &[0xab; 10]),
            (FourCc::VP8L, &[1, 2, 3]), // odd → padded
            (FourCc::EXIF, &[0xee; 4]),
        ]);
        let got: Vec<Chunk> = RiffReader::new(&file)
            .unwrap()
            .map(|c| c.unwrap())
            .collect();
        assert_eq!(got.len(), 3);
        assert_eq!(
            got[0],
            Chunk {
                fourcc: FourCc::VP8X,
                payload: &[0xab; 10]
            }
        );
        assert_eq!(
            got[1],
            Chunk {
                fourcc: FourCc::VP8L,
                payload: &[1, 2, 3][..]
            }
        );
        assert_eq!(
            got[2],
            Chunk {
                fourcc: FourCc::EXIF,
                payload: &[0xee; 4]
            }
        );
    }

    #[test]
    fn empty_chunk_list_yields_nothing() {
        let file = RiffWriter::new().finish();
        assert_eq!(RiffReader::new(&file).unwrap().count(), 0);
    }

    #[test]
    fn rejects_short_header() {
        assert!(RiffReader::new(b"RIFF").is_err());
    }

    #[test]
    fn rejects_bad_magic() {
        let mut file = build(&[(FourCc::VP8L, &[0; 4])]);
        file[0] = b'X';
        assert!(RiffReader::new(&file).is_err());
    }

    #[test]
    fn rejects_non_webp_form() {
        let mut file = build(&[(FourCc::VP8L, &[0; 4])]);
        file[8] = b'A'; // corrupt 'WEBP'
        assert!(RiffReader::new(&file).is_err());
    }

    #[test]
    fn rejects_file_size_past_end() {
        let mut file = build(&[(FourCc::VP8L, &[0; 4])]);
        file[4..8].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
        assert!(RiffReader::new(&file).is_err());
    }

    #[test]
    fn errors_on_chunk_size_exceeding_data() {
        let mut file = build(&[(FourCc::VP8L, &[0; 4])]);
        // Inflate the chunk's declared size beyond the bytes that follow it.
        file[16..20].copy_from_slice(&0xffu32.to_le_bytes());
        let mut reader = RiffReader::new(&file).unwrap();
        assert!(reader.next().unwrap().is_err());
        assert!(reader.next().is_none(), "iteration stops after an error");
    }

    #[test]
    fn errors_on_truncated_trailing_header() {
        // A valid VP8L chunk followed by 3 stray bytes (too few for another header). Hand-build so
        // the RIFF file size includes the stray bytes.
        let mut w = RiffWriter::new();
        w.write_chunk(FourCc::VP8L, &[0; 4]);
        let mut file = w.finish();
        file.extend_from_slice(&[1, 2, 3]);
        let new_size = u32::try_from(file.len() - 8).unwrap();
        file[4..8].copy_from_slice(&new_size.to_le_bytes());
        let results: Vec<_> = RiffReader::new(&file).unwrap().collect();
        assert_eq!(results.len(), 2);
        assert!(results[0].is_ok());
        assert!(results[1].is_err(), "trailing partial header is an error");
    }
}
