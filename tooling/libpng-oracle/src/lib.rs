//! Dev-only differential oracle around a vendored, statically-linked **libpng** (v1.6.43).
//!
//! gamut-png's encoder must produce files the canonical reference reader decodes back to the same
//! pixels. This crate wraps libpng (built from `third_party/libpng` against `third_party/zlib`)
//! behind a small, safe [`decode`] that returns raw samples at the file's native colour type and bit
//! depth, applying no colour transforms — exactly what a pixel-for-pixel cross-check needs.
//!
//! gamut ships no PNG decoder (decoding is out of scope), so libpng is the reference that proves the
//! encoder correct. All `unsafe` FFI is confined to this crate. libpng signals fatal errors through
//! a callback that must not return; for a dev oracle fed the encoder's own output, an error means a
//! real bug, so the callback prints and aborts (no `setjmp` gymnastics).

#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case)]

use std::os::raw::{c_char, c_void};

mod sys {
    #![allow(dead_code)]
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

/// PNG colour type: greyscale (no alpha).
pub const COLOR_GRAY: u8 = 0;
/// PNG colour type: truecolour RGB.
pub const COLOR_RGB: u8 = 2;
/// PNG colour type: palette (indexed).
pub const COLOR_PALETTE: u8 = 3;
/// PNG colour type: greyscale with alpha.
pub const COLOR_GRAY_ALPHA: u8 = 4;
/// PNG colour type: truecolour with alpha (RGBA).
pub const COLOR_RGBA: u8 = 6;

/// A PNG decoded by libpng into raw samples.
pub struct DecodedImage {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Bits per sample as stored (1, 2, 4, 8, or 16).
    pub bit_depth: u8,
    /// PNG colour-type code (one of the `COLOR_*` constants).
    pub color_type: u8,
    /// Bytes per row of [`pixels`](Self::pixels).
    pub rowbytes: usize,
    /// Raw sample rows, tightly packed (`rowbytes * height`). Sub-byte depths are unpacked to one
    /// byte per sample (values unscaled); 16-bit samples are big-endian, as in the file.
    pub pixels: Vec<u8>,
}

/// Cursor over the in-memory PNG, handed to libpng's read callback via its IO pointer.
struct ReadCursor {
    data: *const u8,
    len: usize,
    pos: usize,
}

unsafe extern "C" fn read_callback(png: sys::png_structp, out: sys::png_bytep, count: usize) {
    unsafe {
        let cursor = &mut *(sys::png_get_io_ptr(png) as *mut ReadCursor);
        if cursor.pos + count > cursor.len {
            eprintln!("libpng-oracle: read past end of input");
            std::process::abort();
        }
        std::ptr::copy_nonoverlapping(cursor.data.add(cursor.pos), out, count);
        cursor.pos += count;
    }
}

unsafe extern "C" fn error_callback(_png: sys::png_structp, msg: sys::png_const_charp) {
    let text = if msg.is_null() {
        "unknown".to_string()
    } else {
        unsafe { std::ffi::CStr::from_ptr(msg).to_string_lossy().into_owned() }
    };
    eprintln!("libpng-oracle: fatal libpng error: {text}");
    std::process::abort();
}

unsafe extern "C" fn warn_callback(_png: sys::png_structp, _msg: sys::png_const_charp) {}

/// Decodes a PNG byte stream with libpng into raw samples (no colour transforms).
///
/// Aborts the process if libpng reports the stream is malformed — for this dev oracle the input is
/// always the encoder's own output, so a failure is a genuine bug to surface loudly.
#[must_use]
pub fn decode(bytes: &[u8]) -> DecodedImage {
    unsafe {
        let version = sys::PNG_LIBPNG_VER_STRING.as_ptr() as *const c_char;
        let png = sys::png_create_read_struct(
            version,
            std::ptr::null_mut(),
            Some(error_callback),
            Some(warn_callback),
        );
        assert!(!png.is_null(), "png_create_read_struct failed");
        let mut info = sys::png_create_info_struct(png);
        assert!(!info.is_null(), "png_create_info_struct failed");

        let mut cursor = ReadCursor {
            data: bytes.as_ptr(),
            len: bytes.len(),
            pos: 0,
        };
        sys::png_set_read_fn(
            png,
            (&raw mut cursor).cast::<c_void>(),
            Some(read_callback),
        );
        // Treat recoverable (benign) errors in ancillary chunks as warnings: gamut-png frames
        // metadata chunks (eXIf/iCCP/...) but does not validate their payloads, so an oracle
        // checking the *image* should not abort on third-party metadata content. Critical errors
        // (IHDR, IDAT, CRC) still abort.
        sys::png_set_benign_errors(png, 1);
        sys::png_read_info(png, info);

        let width = sys::png_get_image_width(png, info);
        let height = sys::png_get_image_height(png, info);
        let bit_depth = sys::png_get_bit_depth(png, info) as u8;
        let color_type = sys::png_get_color_type(png, info) as u8;

        // Unpack 1/2/4-bit samples to one byte each (values left unscaled); leave 16-bit big-endian.
        if bit_depth < 8 {
            sys::png_set_packing(png);
        }
        sys::png_set_interlace_handling(png); // de-interlace within png_read_image (1 pass if none)
        sys::png_read_update_info(png, info);

        let rowbytes = sys::png_get_rowbytes(png, info) as usize;
        let mut pixels = vec![0u8; rowbytes * height as usize];
        let mut rows: Vec<sys::png_bytep> = (0..height as usize)
            .map(|y| pixels.as_mut_ptr().add(y * rowbytes))
            .collect();
        sys::png_read_image(png, rows.as_mut_ptr());
        sys::png_read_end(png, std::ptr::null_mut());

        let mut png = png;
        sys::png_destroy_read_struct(&raw mut png, &raw mut info, std::ptr::null_mut());

        DecodedImage {
            width,
            height,
            bit_depth,
            color_type,
            rowbytes,
            pixels,
        }
    }
}

/// Decodes a PNG to 8-bit RGBA via libpng's simplified API, resolving palette and tRNS to actual
/// colours. Returns `(width, height, rgba)`. Useful for verifying that palette entries and
/// transparency resolve to the colours the encoder intended.
#[must_use]
pub fn decode_rgba8(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    unsafe {
        let mut image: sys::png_image = std::mem::zeroed();
        image.version = sys::PNG_IMAGE_VERSION;
        let ok = sys::png_image_begin_read_from_memory(
            &raw mut image,
            bytes.as_ptr().cast::<c_void>(),
            bytes.len(),
        );
        assert!(ok != 0, "png_image_begin_read_from_memory failed");
        image.format = sys::PNG_FORMAT_RGBA;
        let (width, height) = (image.width, image.height);
        let mut rgba = vec![0u8; width as usize * height as usize * 4];
        let ok = sys::png_image_finish_read(
            &raw mut image,
            std::ptr::null(),
            rgba.as_mut_ptr().cast::<c_void>(),
            0, // row stride: 0 = packed (width * 4)
            std::ptr::null_mut(),
        );
        assert!(ok != 0, "png_image_finish_read failed");
        (width, height, rgba)
    }
}

/// The libpng version number the oracle links against (e.g. `10643` for 1.6.43).
#[must_use]
pub fn version() -> u32 {
    unsafe { sys::png_access_version_number() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_libpng_1_6() {
        // Confirms the static lib, bindgen FFI, and link line are all wired correctly.
        assert!(version() >= 10600, "expected libpng 1.6.x, got {}", version());
    }
}
