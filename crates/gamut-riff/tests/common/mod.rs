//! Shared test-support for the libwebp differential container oracle.
//!
//! gamut-riff codes no bitstream, so the container *is* the whole of what it must get right, and
//! libwebp's demuxer (`WebPDemux`, via the `libwebp-sys2` dev-dependency) is the reference parser
//! for it: it walks the RIFF chunk structure and reports each chunk and the canvas. All `unsafe`
//! FFI is confined to this module behind safe wrappers, so the shipped `gamut-riff` library stays
//! `#![forbid(unsafe_code)]` (the `forbid` is per-crate and does not cover integration-test crates).
//!
//! The crate cannot *produce* a codestream, so a real one is borrowed from libwebp's own encoder:
//! every test image starts life as a libwebp-encoded file, which gives both directions of the
//! differential — gamut-riff parsing what libwebp wrote, and libwebp parsing what gamut-riff
//! rewrapped.

use std::ffi::c_void;
use std::slice;

/// Encodes `width` x `height` RGB pixels to a lossless WebP file using libwebp's own encoder.
///
/// The result is a genuine simple-format file with a real `VP8L` codestream — the raw material the
/// container tests rewrap. Panics if libwebp declines, which would mean the harness is broken.
#[must_use]
pub fn libwebp_encode_lossless(rgb: &[u8], width: u32, height: u32) -> Vec<u8> {
    assert_eq!(rgb.len(), (width * height * 3) as usize, "RGB buffer size");
    let mut out: *mut u8 = std::ptr::null_mut();
    // SAFETY: `rgb` is a valid slice of the asserted length with a 3-byte-per-pixel stride; `out`
    // receives an allocation owned by libwebp, freed below.
    let len = unsafe {
        libwebp_sys::WebPEncodeLosslessRGB(
            rgb.as_ptr(),
            width as i32,
            height as i32,
            (width * 3) as i32,
            &mut out,
        )
    };
    assert!(len > 0 && !out.is_null(), "libwebp lossless encode failed");
    // SAFETY: libwebp reports `len` valid bytes at `out`.
    let file = unsafe { slice::from_raw_parts(out, len) }.to_vec();
    // SAFETY: `out` came from libwebp's allocator.
    unsafe { libwebp_sys::WebPFree(out.cast::<c_void>()) };
    file
}

/// One chunk as libwebp's demuxer reports it: the FourCC and the payload bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleChunk {
    /// The chunk's four-character code.
    pub fourcc: [u8; 4],
    /// The chunk payload, excluding the header and any pad byte.
    pub payload: Vec<u8>,
}

/// What libwebp's demuxer makes of a file: its canvas and the chunks it could enumerate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleView {
    /// Canvas width libwebp reports, in pixels.
    pub canvas_width: u32,
    /// Canvas height libwebp reports, in pixels.
    pub canvas_height: u32,
    /// The `ICCP`, `EXIF`, and `XMP ` chunks, in that order, for those the file carries.
    ///
    /// libwebp's chunk iterator covers exactly the metadata chunks; the bitstream and `ALPH` are
    /// reached through its *frame* iterator instead, so they are not listed here.
    pub metadata: Vec<OracleChunk>,
}

/// Parses `file` with libwebp's demuxer, returning `None` if libwebp rejects it.
///
/// A `None` here is itself an oracle result: it means the reference parser considers the bytes
/// malformed, which a test can assert against gamut-riff's own verdict.
#[must_use]
pub fn libwebp_demux(file: &[u8]) -> Option<OracleView> {
    let data = libwebp_sys::WebPData {
        bytes: file.as_ptr(),
        size: file.len(),
    };
    // SAFETY: `data` borrows `file` for the duration of the call and of the demuxer's life, which
    // ends at the `WebPDemuxDelete` below.
    let dmux = unsafe { libwebp_sys::WebPDemux(&data) };
    if dmux.is_null() {
        return None;
    }
    // SAFETY: `dmux` is a live demuxer; these getters only read from it.
    let (canvas_width, canvas_height) = unsafe {
        (
            libwebp_sys::WebPDemuxGetI(dmux, libwebp_sys::WEBP_FF_CANVAS_WIDTH),
            libwebp_sys::WebPDemuxGetI(dmux, libwebp_sys::WEBP_FF_CANVAS_HEIGHT),
        )
    };

    let mut metadata = Vec::new();
    for tag in [b"ICCP", b"EXIF", b"XMP "] {
        // SAFETY: zeroed is a valid initial state for the iterator; libwebp fills it in and we
        // release it before it goes out of scope.
        let mut iter: libwebp_sys::WebPChunkIterator = unsafe { std::mem::zeroed() };
        // SAFETY: `tag` is a 4-byte ASCII FourCC; libwebp copies it and does not retain the pointer.
        let found = unsafe {
            libwebp_sys::WebPDemuxGetChunk(dmux, tag.as_ptr().cast::<i8>(), 1, &mut iter)
        };
        if found != 0 {
            // SAFETY: on success the iterator's `chunk` points at `size` bytes inside `file`.
            let payload =
                unsafe { slice::from_raw_parts(iter.chunk.bytes, iter.chunk.size) }.to_vec();
            metadata.push(OracleChunk {
                fourcc: *tag,
                payload,
            });
            // SAFETY: releases the iterator libwebp initialised above.
            unsafe { libwebp_sys::WebPDemuxReleaseChunkIterator(&mut iter) };
        }
    }

    // SAFETY: `dmux` was returned by `WebPDemux` and is not used after this point.
    unsafe { libwebp_sys::WebPDemuxDelete(dmux) };
    Some(OracleView {
        canvas_width,
        canvas_height,
        metadata,
    })
}

/// A deterministic RGB test image with enough variation that a codestream is non-trivial.
#[must_use]
pub fn rgb_image(width: u32, height: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&[
                (x * 7 % 256) as u8,
                (y * 11 % 256) as u8,
                ((x + y) * 13 % 256) as u8,
            ]);
        }
    }
    pixels
}
