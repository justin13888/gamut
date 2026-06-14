//! The Photoshop Image Resource Block (`8BIM`) carrier for legacy IPTC-IIM.
//!
//! Adobe Photoshop stores image resources as a sequence of blocks, each consisting of:
//!
//! 1. the ASCII signature `8BIM` (4 octets),
//! 2. a 2-octet big-endian resource id,
//! 3. a Pascal-string resource name — a length octet followed by that many octets, the whole field
//!    padded with a trailing `0x00` to an even length,
//! 4. a 4-octet big-endian data length, and
//! 5. the resource data, padded with a trailing `0x00` to an even length.
//!
//! Legacy IPTC-IIM lives in the resource with id `0x0404` ([`IPTC_RESOURCE_ID`]). This module
//! parses and serializes the block stream; the JPEG `APP13` `Photoshop 3.0\0` wrapper that carries
//! it in a file is the container's concern, not this crate's.

use gamut_core::{Error, Result};

use crate::charset::{decode_latin1, encode_latin1};

/// The four-octet signature that introduces every image-resource block.
const SIGNATURE: &[u8; 4] = b"8BIM";

/// The Photoshop image-resource id under which legacy IPTC-IIM datasets are stored (`0x0404`).
pub const IPTC_RESOURCE_ID: u16 = 0x0404;

/// One Photoshop image-resource block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrbBlock {
    /// The 2-octet resource id ([`IPTC_RESOURCE_ID`] is the IPTC-IIM block).
    pub resource_id: u16,
    /// The optional Pascal-string resource name (usually empty).
    pub name: String,
    /// The raw resource data (for `0x0404`, the IIM dataset stream — see [`crate::iim`]).
    pub data: Vec<u8>,
}

/// A parsed Photoshop image resource: the `8BIM` block stream, in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PhotoshopIrb {
    /// The image-resource blocks, in file order.
    pub blocks: Vec<IrbBlock>,
}

impl PhotoshopIrb {
    /// Builds an image resource holding a single `0x0404` block with the given IIM dataset stream.
    #[must_use]
    pub fn with_iptc(iim_stream: Vec<u8>) -> Self {
        Self {
            blocks: vec![IrbBlock {
                resource_id: IPTC_RESOURCE_ID,
                name: String::new(),
                data: iim_stream,
            }],
        }
    }

    /// The data of the `0x0404` (IPTC-IIM) resource, if present.
    #[must_use]
    pub fn iptc_iim(&self) -> Option<&[u8]> {
        self.blocks
            .iter()
            .find(|b| b.resource_id == IPTC_RESOURCE_ID)
            .map(|b| b.data.as_slice())
    }

    /// Parses a Photoshop image-resource (`8BIM`) block stream.
    ///
    /// Every offset is bounds-checked. The length-prefixed name and data fields and their even
    /// padding are handled per the block layout in the module docs.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if a block signature is not `8BIM` or the stream is
    /// truncated.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut blocks = Vec::new();
        let mut pos = 0usize;
        while pos < data.len() {
            let sig = data
                .get(pos..pos + 4)
                .ok_or(Error::InvalidInput("IPTC IRB: truncated block signature"))?;
            if sig != SIGNATURE {
                return Err(Error::InvalidInput("IPTC IRB: bad 8BIM block signature"));
            }
            let id = data
                .get(pos + 4..pos + 6)
                .ok_or(Error::InvalidInput("IPTC IRB: truncated resource id"))?;
            let resource_id = u16::from_be_bytes([id[0], id[1]]);

            let name_len = usize::from(
                *data
                    .get(pos + 6)
                    .ok_or(Error::InvalidInput("IPTC IRB: truncated resource name"))?,
            );
            let name_start = pos + 7;
            let name_end = name_start
                .checked_add(name_len)
                .ok_or(Error::InvalidInput("IPTC IRB: name length overflow"))?;
            let name_bytes = data
                .get(name_start..name_end)
                .ok_or(Error::InvalidInput("IPTC IRB: truncated resource name"))?;
            let name = decode_latin1(name_bytes);
            // The name field (length octet + name octets) is padded to an even length.
            let size_start = name_end + (1 + name_len) % 2;

            let size_bytes = data
                .get(size_start..size_start + 4)
                .ok_or(Error::InvalidInput("IPTC IRB: truncated resource size"))?;
            let size =
                u32::from_be_bytes([size_bytes[0], size_bytes[1], size_bytes[2], size_bytes[3]])
                    as usize;
            let data_start = size_start + 4;
            let data_end = data_start
                .checked_add(size)
                .ok_or(Error::InvalidInput("IPTC IRB: size overflow"))?;
            let block_data = data
                .get(data_start..data_end)
                .ok_or(Error::InvalidInput("IPTC IRB: truncated resource data"))?;
            blocks.push(IrbBlock {
                resource_id,
                name,
                data: block_data.to_vec(),
            });
            // The data field is padded to an even length. Advancing to `data_end` first (always
            // past `pos`) keeps the loop strictly forward-progressing.
            pos = data_end;
            if size % 2 == 1 {
                pos += 1;
            }
        }
        Ok(Self { blocks })
    }

    /// Serializes the image resource back to an `8BIM` block stream.
    ///
    /// The output round-trips through [`PhotoshopIrb::parse`] byte-for-byte, including the even
    /// padding of odd-length name and data fields.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if a resource name exceeds 255 octets, is not representable
    /// in Latin-1, or a resource's data exceeds the 4 GiB the length field can encode.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        for b in &self.blocks {
            out.extend_from_slice(SIGNATURE);
            out.extend_from_slice(&b.resource_id.to_be_bytes());

            let name_bytes = encode_latin1(&b.name)?;
            let name_len = u8::try_from(name_bytes.len())
                .map_err(|_| Error::InvalidInput("IPTC IRB: resource name too long"))?;
            out.push(name_len);
            out.extend_from_slice(&name_bytes);
            if (1 + name_bytes.len()) % 2 == 1 {
                out.push(0);
            }

            let size = u32::try_from(b.data.len())
                .map_err(|_| Error::InvalidInput("IPTC IRB: resource data too large"))?;
            out.extend_from_slice(&size.to_be_bytes());
            out.extend_from_slice(&b.data);
            if b.data.len() % 2 == 1 {
                out.push(0);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(resource_id: u16, name: &str, data: &[u8]) -> IrbBlock {
        IrbBlock {
            resource_id,
            name: name.to_owned(),
            data: data.to_vec(),
        }
    }

    #[test]
    fn parse_empty_name_block() {
        // 8BIM, 0x0404, name len 0 (+1 pad), size 2, "hi".
        let bytes = [
            b'8', b'B', b'I', b'M', 0x04, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, b'h', b'i',
        ];
        let irb = PhotoshopIrb::parse(&bytes).unwrap();
        assert_eq!(irb.blocks, vec![block(0x0404, "", b"hi")]);
        assert_eq!(irb.iptc_iim(), Some(&b"hi"[..]));
    }

    #[test]
    fn roundtrip_even_and_odd_padding() {
        // Odd-length name ("ab" -> field 1+2=3, padded) and odd-length data (1 octet, padded).
        for irb in [
            PhotoshopIrb {
                blocks: vec![block(0x0404, "", b"hi")], // even name field, even data
            },
            PhotoshopIrb {
                blocks: vec![block(0x040F, "name", b"odd")], // even name field, odd data
            },
            PhotoshopIrb {
                blocks: vec![block(0x0404, "ab", b"data")], // odd name field, even data
            },
            PhotoshopIrb {
                blocks: vec![block(0x0BB7, "x", b"y")], // even name field, odd data
            },
        ] {
            let bytes = irb.encode().unwrap();
            assert_eq!(bytes.len() % 2, 0, "blocks must be even-aligned");
            assert_eq!(PhotoshopIrb::parse(&bytes).unwrap(), irb);
        }
    }

    #[test]
    fn locate_iptc_among_several_blocks() {
        let irb = PhotoshopIrb {
            blocks: vec![
                block(0x03ED, "", &[0, 1]), // even size 2, followed by another block
                block(0x040F, "", b"odd!"), // resolution info
                block(0x0404, "", b"IIM stream"),
                block(0x040C, "", &[9, 9, 9]), // thumbnail (odd size, padded)
            ],
        };
        let bytes = irb.encode().unwrap();
        let parsed = PhotoshopIrb::parse(&bytes).unwrap();
        assert_eq!(parsed, irb);
        assert_eq!(parsed.iptc_iim(), Some(&b"IIM stream"[..]));
    }

    #[test]
    fn iptc_iim_absent_returns_none() {
        let irb = PhotoshopIrb {
            blocks: vec![block(0x03ED, "", &[0, 1, 2, 3])],
        };
        assert_eq!(irb.iptc_iim(), None);
    }

    #[test]
    fn with_iptc_builds_0x0404_block() {
        let irb = PhotoshopIrb::with_iptc(vec![1, 2, 3]);
        assert_eq!(irb.iptc_iim(), Some(&[1, 2, 3][..]));
        assert_eq!(PhotoshopIrb::parse(&irb.encode().unwrap()).unwrap(), irb);
    }

    #[test]
    fn parse_rejects_bad_signature_and_truncation() {
        assert!(PhotoshopIrb::parse(b"9BIM\x04\x04\x00\x00\x00\x00\x00\x00").is_err());
        // Truncated before the size field.
        assert!(PhotoshopIrb::parse(b"8BIM\x04\x04\x00").is_err());
        // Size declares more data than present.
        assert!(
            PhotoshopIrb::parse(&[
                b'8', b'B', b'I', b'M', 0x04, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x09, b'x'
            ])
            .is_err()
        );
    }
}
