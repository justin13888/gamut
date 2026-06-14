//! Dev-only differential oracle around a vendored, statically-linked **zlib** (v1.3.1).
//!
//! gamut-deflate's DEFLATE/zlib *encoder* must produce streams that the canonical reference
//! inflater decodes back to the original bytes. This crate wraps a zlib built from the
//! `third_party/zlib` submodule behind a small, safe API: [`inflate_raw`] (bare DEFLATE,
//! RFC 1951), [`inflate_zlib`] (zlib-wrapped, RFC 1950), [`compress`] (zlib's own compressor, used
//! only as a size baseline), and the [`adler32`]/[`crc32`] checksums.
//!
//! gamut ships no decoder — PNG/DEFLATE decoding is out of scope — so this reference inflater is how
//! the encoder is proven correct. All `unsafe` FFI is confined to this crate.

#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case)]

use std::os::raw::{c_char, c_int};

mod sys {
    #![allow(dead_code)]
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

/// Inflates a bare DEFLATE stream (RFC 1951, no zlib header/trailer).
///
/// # Errors
///
/// Returns a message if the stream is malformed or truncated.
pub fn inflate_raw(data: &[u8]) -> Result<Vec<u8>, String> {
    // A negative windowBits selects raw deflate (no zlib wrapper).
    unsafe { inflate_impl(data, -15) }
}

/// Inflates a zlib-wrapped DEFLATE stream (RFC 1950).
///
/// # Errors
///
/// Returns a message if the stream is malformed, truncated, or fails its Adler-32 check.
pub fn inflate_zlib(data: &[u8]) -> Result<Vec<u8>, String> {
    unsafe { inflate_impl(data, 15) }
}

unsafe fn inflate_impl(data: &[u8], window_bits: c_int) -> Result<Vec<u8>, String> {
    let mut strm: sys::z_stream = unsafe { std::mem::zeroed() };
    let rc = unsafe {
        sys::inflateInit2_(
            &mut strm,
            window_bits,
            sys::ZLIB_VERSION.as_ptr() as *const c_char,
            std::mem::size_of::<sys::z_stream>() as c_int,
        )
    };
    if rc != sys::Z_OK as c_int {
        return Err(format!("inflateInit2_ failed: {rc}"));
    }
    strm.next_in = data.as_ptr() as *mut u8;
    strm.avail_in = data.len() as u32;
    let mut out = Vec::new();
    let mut chunk = vec![0u8; 1 << 16];
    let result = loop {
        strm.next_out = chunk.as_mut_ptr();
        strm.avail_out = chunk.len() as u32;
        let rc = unsafe { sys::inflate(&mut strm, sys::Z_NO_FLUSH as c_int) };
        let produced = chunk.len() - strm.avail_out as usize;
        out.extend_from_slice(&chunk[..produced]);
        if rc == sys::Z_STREAM_END as c_int {
            break Ok(());
        }
        if rc != sys::Z_OK as c_int {
            break Err(format!("inflate failed: {rc}"));
        }
        if produced == 0 && strm.avail_in == 0 {
            break Err("inflate: truncated stream".to_string());
        }
    };
    unsafe { sys::inflateEnd(&mut strm) };
    result.map(|()| out)
}

/// Compresses `data` with zlib's own compressor at `level` (0–9), zlib-wrapped (RFC 1950).
///
/// Used only as a *size baseline* for gamut-deflate's space-efficiency tests.
///
/// # Errors
///
/// Returns a message if zlib reports a failure.
pub fn compress(data: &[u8], level: i32) -> Result<Vec<u8>, String> {
    let bound = unsafe { sys::compressBound(data.len() as u64) };
    let mut out = vec![0u8; bound as usize];
    let mut out_len: u64 = bound;
    let rc = unsafe {
        sys::compress2(
            out.as_mut_ptr(),
            &mut out_len,
            data.as_ptr(),
            data.len() as u64,
            level as c_int,
        )
    };
    if rc != sys::Z_OK as c_int {
        return Err(format!("compress2 failed: {rc}"));
    }
    out.truncate(out_len as usize);
    Ok(out)
}

/// zlib's Adler-32 (pass `seed = 1` for a fresh checksum). Cross-checks gamut-deflate's own impl.
#[must_use]
pub fn adler32(seed: u32, data: &[u8]) -> u32 {
    unsafe { sys::adler32(seed as u64, data.as_ptr(), data.len() as u32) as u32 }
}

/// zlib's CRC-32 (pass `seed = 0` for a fresh checksum).
#[must_use]
pub fn crc32(seed: u32, data: &[u8]) -> u32 {
    unsafe { sys::crc32(seed as u64, data.as_ptr(), data.len() as u32) as u32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compress_then_inflate_round_trips() {
        let data = b"hello hello hello world, the quick brown fox jumps over it".repeat(10);
        let z = compress(&data, 6).expect("compress");
        assert_eq!(inflate_zlib(&z).expect("inflate"), data);
    }

    #[test]
    fn adler32_known_vector() {
        // Adler-32("Wikipedia") = 0x11E60398 (RFC 1950 worked example lineage).
        assert_eq!(adler32(1, b"Wikipedia"), 0x11E6_0398);
    }
}
