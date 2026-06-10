//! Shared test-support for the libwebp differential oracle.
//!
//! libwebp (via the `libwebp-sys2` dev-dependency) is gamut-webp's reference oracle: lossless coding
//! must round-trip bit-exactly through it, and a libwebp-encoded file must decode identically in
//! gamut. All `unsafe` FFI is confined to this module behind safe wrappers, so the shipped
//! `gamut-webp` library stays `#![forbid(unsafe_code)]` (the `forbid` is per-crate and does not
//! cover these integration-test crates).
//!
//! These wrappers are the harness the per-milestone differential tests build on; today the only
//! test is a libwebp self-round-trip that proves the harness (FFI + linked libwebp) works before any
//! gamut codec exists. As the VP8L/VP8 paths land, tests here will compare gamut's encoder output to
//! libwebp's decode (and vice versa).

use std::ffi::{c_int, c_void};
use std::slice;

/// A YUV 4:2:0 image decoded by libwebp: a full-resolution luma plane and half-resolution chroma.
pub struct DecodedYuv {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Luma plane, `width * height` bytes, row-major.
    pub y: Vec<u8>,
    /// Cb (U) plane, `ceil(width/2) * ceil(height/2)` bytes.
    pub u: Vec<u8>,
    /// Cr (V) plane.
    pub v: Vec<u8>,
}

/// Decodes a WebP file with libwebp into YUV 4:2:0 planes (cropped to the visible dimensions, with the
/// libwebp row strides removed). This is the bit-exact comparison surface for the lossy codec: a VP8
/// bitstream decodes to the same integer YUV in any conformant decoder.
#[must_use]
pub fn libwebp_decode_yuv(webp: &[u8]) -> DecodedYuv {
    let mut width: c_int = 0;
    let mut height: c_int = 0;
    let mut u_ptr: *mut u8 = std::ptr::null_mut();
    let mut v_ptr: *mut u8 = std::ptr::null_mut();
    let mut stride: c_int = 0;
    let mut uv_stride: c_int = 0;
    // SAFETY: `webp` is a valid slice; the out-params receive the dimensions, the U/V plane pointers
    // (into the single returned allocation), and the row strides. The Y pointer owns the allocation.
    let y_ptr = unsafe {
        libwebp_sys::WebPDecodeYUV(
            webp.as_ptr(),
            webp.len(),
            &mut width,
            &mut height,
            &mut u_ptr,
            &mut v_ptr,
            &mut stride,
            &mut uv_stride,
        )
    };
    assert!(!y_ptr.is_null(), "libwebp YUV decode failed");
    let (w, h) = (width as usize, height as usize);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    // SAFETY: each plane is valid for `rows * stride` bytes; we copy out `pw` per row, discarding pad.
    let copy_plane = |ptr: *const u8, row_stride: usize, pw: usize, ph: usize| {
        let mut out = vec![0u8; pw * ph];
        for row in 0..ph {
            let src = unsafe { slice::from_raw_parts(ptr.add(row * row_stride), pw) };
            out[row * pw..row * pw + pw].copy_from_slice(src);
        }
        out
    };
    let y = copy_plane(y_ptr, stride as usize, w, h);
    let u = copy_plane(u_ptr, uv_stride as usize, cw, ch);
    let v = copy_plane(v_ptr, uv_stride as usize, cw, ch);
    // SAFETY: `y_ptr` is the head of the single libwebp allocation backing all three planes.
    unsafe { libwebp_sys::WebPFree(y_ptr.cast::<c_void>()) };
    DecodedYuv {
        width: width as u32,
        height: height as u32,
        y,
        u,
        v,
    }
}

/// An RGBA image decoded by libwebp: interleaved 8-bit `R,G,B,A` pixels plus dimensions.
pub struct DecodedRgba {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Interleaved 8-bit RGBA pixels, `width * height * 4` bytes in scan order.
    pub rgba: Vec<u8>,
}

/// Encodes interleaved 8-bit RGBA with libwebp's **lossless** coder, returning the WebP file bytes.
///
/// `rgba` must be exactly `width * height * 4` bytes. Panics (these are tests) if the buffer size is
/// wrong or libwebp reports an error.
#[must_use]
pub fn libwebp_encode_lossless_rgba(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let expected = width as usize * height as usize * 4;
    assert_eq!(
        rgba.len(),
        expected,
        "RGBA buffer is not width*height*4 bytes"
    );

    let mut out_ptr: *mut u8 = std::ptr::null_mut();
    let stride = width as c_int * 4;
    // SAFETY: `rgba` is valid for `expected` bytes; `out_ptr` receives a libwebp-allocated buffer
    // that we copy out and free below. The dimensions/stride describe `rgba` exactly.
    let size = unsafe {
        libwebp_sys::WebPEncodeLosslessRGBA(
            rgba.as_ptr(),
            width as c_int,
            height as c_int,
            stride,
            &mut out_ptr,
        )
    };
    assert!(
        size != 0 && !out_ptr.is_null(),
        "libwebp lossless encode failed"
    );
    // SAFETY: libwebp guarantees `out_ptr` points to `size` valid bytes.
    let bytes = unsafe { slice::from_raw_parts(out_ptr, size) }.to_vec();
    // SAFETY: `out_ptr` was allocated by libwebp and must be released with WebPFree.
    unsafe { libwebp_sys::WebPFree(out_ptr.cast::<c_void>()) };
    bytes
}

/// Reads the canvas dimensions of a WebP file with libwebp, or `None` if it is not a valid WebP.
#[must_use]
pub fn libwebp_get_info(webp: &[u8]) -> Option<(u32, u32)> {
    let mut width: c_int = 0;
    let mut height: c_int = 0;
    // SAFETY: `webp` is a valid slice of `webp.len()` bytes; `width`/`height` are out-params.
    let ok =
        unsafe { libwebp_sys::WebPGetInfo(webp.as_ptr(), webp.len(), &mut width, &mut height) };
    if ok == 0 {
        return None;
    }
    Some((width as u32, height as u32))
}

/// Decodes a WebP file with libwebp into interleaved 8-bit RGBA. Panics (these are tests) if libwebp
/// rejects the input.
#[must_use]
pub fn libwebp_decode_rgba(webp: &[u8]) -> DecodedRgba {
    let mut width: c_int = 0;
    let mut height: c_int = 0;
    // SAFETY: `webp` is a valid slice; `width`/`height` are out-params; the returned pointer is a
    // libwebp-allocated RGBA buffer (or null on error).
    let ptr =
        unsafe { libwebp_sys::WebPDecodeRGBA(webp.as_ptr(), webp.len(), &mut width, &mut height) };
    assert!(!ptr.is_null(), "libwebp decode failed");
    let len = width as usize * height as usize * 4;
    // SAFETY: libwebp returned a buffer of `width * height * 4` bytes.
    let rgba = unsafe { slice::from_raw_parts(ptr, len) }.to_vec();
    // SAFETY: `ptr` was allocated by libwebp and must be released with WebPFree.
    unsafe { libwebp_sys::WebPFree(ptr.cast::<c_void>()) };
    DecodedRgba {
        width: width as u32,
        height: height as u32,
        rgba,
    }
}

/// Generates a deterministic, fully-opaque RGBA test image with enough structure and variation to
/// exercise the codec. Alpha is held at 255 so libwebp's default lossless mode (which may rewrite
/// the RGB of transparent pixels) preserves every channel bit-exactly.
#[must_use]
pub fn pattern_rgba(width: u32, height: u32) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            rgba.push(((x * 7 + y * 3) & 0xff) as u8); // R
            rgba.push(((x ^ (y * 5)) & 0xff) as u8); // G
            rgba.push(((x * x + y) & 0xff) as u8); // B
            rgba.push(0xff); // A (opaque)
        }
    }
    rgba
}
