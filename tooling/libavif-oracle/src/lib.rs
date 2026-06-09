//! Dev-only differential oracle around a vendored, statically-linked **libavif**.
//!
//! gamut's AVIF encoder must produce containers that a real AVIF reader decodes back to the same
//! pixels the AV1 layer reconstructed. This crate parses + decodes a full `.avif` byte stream with
//! libavif (dav1d backend) and hands back the decoded YUV planes, so the end-to-end roundtrip test
//! can compare them to the source (lossless) or the encoder's reconstruction (lossy) — without
//! depending on an `avifdec` binary being installed. The decoded planes are the exact bytes the AV1
//! decoder produced (no RGB color conversion), mirroring the old `avifdec` Y4M path. The C libraries
//! are built from the `third_party/libavif` and `third_party/dav1d` git submodules by `build.rs`.
//!
//! All `unsafe` FFI is confined here behind a single safe entry point, [`decode_avif`].

#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case)]

use std::ptr;

mod sys {
    #![allow(dead_code)]
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

/// A decoded 8-bit planar image: one tightly packed `width * height` plane per component.
///
/// gamut emits 4:4:4 identity-matrix stills, so the three planes are full-resolution and carry
/// `[Y, U, V]` (which under the identity matrix are `G, B, R`).
pub struct DecodedImage {
    /// Luma width in pixels.
    pub width: u32,
    /// Luma height in pixels.
    pub height: u32,
    /// `[Y, U, V]` planes, each in raster order with no row padding.
    pub planes: [Vec<u8>; 3],
}

/// Decodes the first frame of an AVIF file into its 8-bit 4:4:4 YUV planes.
///
/// # Errors
///
/// Returns a message (including libavif's own result string) if the file cannot be parsed or
/// decoded, or if the decoded image is not 8-bit 4:4:4 (the only form gamut emits).
pub fn decode_avif(avif: &[u8]) -> Result<DecodedImage, String> {
    // SAFETY: the decoder and image handles below are created and destroyed in matched pairs on
    // every return path; pointers passed to libavif stay valid for each call's duration.
    unsafe { decode_inner(avif) }
}

unsafe fn decode_inner(avif: &[u8]) -> Result<DecodedImage, String> {
    unsafe {
        let decoder = sys::avifDecoderCreate();
        if decoder.is_null() {
            return Err("avifDecoderCreate returned null".into());
        }
        let image = sys::avifImageCreateEmpty();
        if image.is_null() {
            sys::avifDecoderDestroy(decoder);
            return Err("avifImageCreateEmpty returned null".into());
        }

        let result = sys::avifDecoderReadMemory(decoder, image, avif.as_ptr(), avif.len());
        let out = if result == sys::AVIF_RESULT_OK {
            extract(&*image)
        } else {
            Err(format!(
                "avifDecoderReadMemory failed: {}",
                result_str(result)
            ))
        };

        sys::avifImageDestroy(image);
        sys::avifDecoderDestroy(decoder);
        out
    }
}

/// Copies the three YUV planes out of a decoded `avifImage` into owned, unpadded buffers.
unsafe fn extract(image: &sys::avifImage) -> Result<DecodedImage, String> {
    if image.depth != 8 {
        return Err(format!("expected 8-bit image, got {}-bit", image.depth));
    }
    if image.yuvFormat != sys::AVIF_PIXEL_FORMAT_YUV444 {
        return Err(format!(
            "expected 4:4:4, got pixel format {}",
            image.yuvFormat
        ));
    }
    let w = image.width as usize;
    let h = image.height as usize;

    // SAFETY: a successfully decoded 4:4:4 image owns three planes of `h` rows; `yuvRowBytes[p]`
    // (>= w) is the stride of plane `p`.
    unsafe {
        let mut planes = [Vec::new(), Vec::new(), Vec::new()];
        for (p, plane) in planes.iter_mut().enumerate() {
            let base = image.yuvPlanes[p];
            if base.is_null() {
                return Err(format!("plane {p} is null"));
            }
            *plane = copy_plane(base, image.yuvRowBytes[p] as usize, w, h);
        }
        let [y, u, v] = planes;
        Ok(DecodedImage {
            width: image.width,
            height: image.height,
            planes: [y, u, v],
        })
    }
}

/// Copies a `w`×`h` 8-bit plane from a strided buffer into a tightly packed `Vec`.
unsafe fn copy_plane(base: *const u8, stride: usize, w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w * h];
    // SAFETY: caller guarantees `base` addresses `h` rows of at least `w` bytes spaced `stride`
    // apart; `out` is exactly `w * h` bytes, so each row copy stays in bounds of both buffers.
    unsafe {
        for row in 0..h {
            let src = base.add(stride * row);
            ptr::copy_nonoverlapping(src, out.as_mut_ptr().add(row * w), w);
        }
    }
    out
}

/// libavif's human-readable string for a result code.
unsafe fn result_str(result: sys::avifResult) -> String {
    unsafe {
        let ptr = sys::avifResultToString(result);
        if ptr.is_null() {
            return format!("avifResult({result})");
        }
        std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }
}
