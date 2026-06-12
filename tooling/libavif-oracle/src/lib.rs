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

mod sys {
    #![allow(dead_code)]
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

/// A decoded planar image: one tightly packed `width * height` plane per component, each sample
/// widened to `u16` (8-bit samples occupy `0..=255`, 10-/12-bit use the wider range).
///
/// gamut emits 4:4:4 stills, so the three planes are full-resolution and carry `[Y, U, V]` (which
/// under the identity matrix are `G, B, R`).
pub struct DecodedImage {
    /// Luma width in pixels.
    pub width: u32,
    /// Luma height in pixels.
    pub height: u32,
    /// Bits per component (8, 10, or 12).
    pub bit_depth: u8,
    /// `[Y, U, V]` planes, each in raster order with no row padding; samples widened to `u16`.
    pub planes: [Vec<u16>; 3],
}

/// Decodes the first frame of an AVIF file into its 4:4:4 YUV planes (8/10/12-bit, widened to `u16`).
///
/// # Errors
///
/// Returns a message (including libavif's own result string) if the file cannot be parsed or
/// decoded, or if the decoded image is not 4:4:4 or not 8/10/12-bit (the forms gamut emits).
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
    let depth = image.depth as u8;
    if !matches!(depth, 8 | 10 | 12) {
        return Err(format!("unexpected bit depth: {depth}-bit"));
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
    // (the byte stride) spaces consecutive rows of plane `p`.
    unsafe {
        let mut planes = [Vec::new(), Vec::new(), Vec::new()];
        for (p, plane) in planes.iter_mut().enumerate() {
            let base = image.yuvPlanes[p];
            if base.is_null() {
                return Err(format!("plane {p} is null"));
            }
            *plane = copy_plane(base, image.yuvRowBytes[p] as usize, w, h, depth);
        }
        let [y, u, v] = planes;
        Ok(DecodedImage {
            width: image.width,
            height: image.height,
            bit_depth: depth,
            planes: [y, u, v],
        })
    }
}

/// Copies a `w`×`h` plane from a strided libavif buffer into a tightly packed `u16` `Vec`. `depth`
/// is the bit depth: at 8 the source samples are bytes (widened to `u16`); at 10/12 they are native-
/// endian `u16` and `byte_stride` is in bytes.
unsafe fn copy_plane(
    base: *const u8,
    byte_stride: usize,
    w: usize,
    h: usize,
    depth: u8,
) -> Vec<u16> {
    let mut out = vec![0u16; w * h];
    // SAFETY: caller guarantees `base` addresses `h` rows of at least `w` samples spaced
    // `byte_stride` bytes apart; each read stays within row `row`'s `w` samples and `out` is exactly
    // `w * h` elements.
    unsafe {
        for row in 0..h {
            let row_base = base.add(byte_stride * row);
            for col in 0..w {
                out[row * w + col] = if depth == 8 {
                    u16::from(*row_base.add(col))
                } else {
                    *row_base.cast::<u16>().add(col)
                };
            }
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
