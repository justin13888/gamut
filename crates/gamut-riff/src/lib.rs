//! Resource Interchange File Format (RIFF) utilities — the chunked container used by WebP.
//!
//! This crate owns the WebP *container*, not the codec: it reads and writes the RIFF chunk
//! structure (`RIFF`/`WEBP` plus `VP8 `/`VP8L`/`VP8X`/… chunks) and leaves the VP8/VP8L bitstream to
//! [`gamut-webp`](https://docs.rs/gamut-webp), mirroring how `gamut-isobmff` backs AVIF/HEIC.
//!
//! Byte layouts follow RFC 9649 (*WebP Image Format*) §2 and the Google *WebP Container*
//! specification in `references/webp/`. The implemented surface — and the extended-format chunks
//! still to come — is tracked in `gamut-webp/STATUS.md` section A.
//!
//! # Example
//!
//! ```
//! use gamut_riff::{RiffReader, WebpChunkId, write_simple_lossless};
//!
//! let file = write_simple_lossless(&[0x2f, 0x01, 0x02]);
//! let chunk = RiffReader::new(&file).unwrap().next().unwrap().unwrap();
//! assert_eq!(WebpChunkId::from(chunk.fourcc), WebpChunkId::Vp8l);
//! assert_eq!(chunk.payload, &[0x2f, 0x01, 0x02]);
//! ```
#![forbid(unsafe_code)]

mod chunk;
mod fourcc;
mod reader;
mod webp;
mod writer;

pub use chunk::{CHUNK_HEADER_LEN, Chunk, ChunkHeader};
pub use fourcc::FourCc;
pub use reader::RiffReader;
pub use webp::{
    VP8X_PAYLOAD_LEN, Vp8xHeader, WebpChunkId, write_extended, write_simple_lossless,
    write_simple_lossy,
};
pub use writer::RiffWriter;
