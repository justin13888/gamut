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

/// Runs libwebp's BT.601 RGB→YUV 4:2:0 conversion — the same one `WebPEncode` applies to lossy input
/// — on interleaved RGBA, returning the YUV planes. Used to pin gamut-color's own conversion against
/// the reference *within a tolerance*: the exact rounding and chroma downsampling are
/// implementation-defined, so this is not a bit-exact surface. Panics (these are tests) on a bad
/// buffer size or a libwebp error.
#[must_use]
pub fn libwebp_rgba_to_yuv(rgba: &[u8], width: u32, height: u32) -> DecodedYuv {
    let expected = width as usize * height as usize * 4;
    assert_eq!(
        rgba.len(),
        expected,
        "RGBA buffer is not width*height*4 bytes"
    );
    // SAFETY: the picture is zero-initialised then Init-filled. With the default `use_argb = 0`,
    // `WebPPictureImportRGBA` runs libwebp's RGBA→YUV conversion in-line (see `Import` →
    // `ImportYUVAFromRGBA` in picture_csp_enc.c), filling the y/u/v planes (valid for `rows * stride`
    // bytes), which we copy out per row before freeing the picture.
    unsafe {
        let mut pic: libwebp_sys::WebPPicture = std::mem::zeroed();
        assert!(
            libwebp_sys::WebPPictureInit(&mut pic) != 0,
            "WebPPictureInit failed"
        );
        pic.width = width as c_int;
        pic.height = height as c_int;
        assert!(
            libwebp_sys::WebPPictureImportRGBA(&mut pic, rgba.as_ptr(), width as c_int * 4) != 0,
            "WebPPictureImportRGBA failed"
        );
        let (w, h) = (width as usize, height as usize);
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let copy_plane = |ptr: *const u8, row_stride: usize, pw: usize, ph: usize| {
            let mut out = vec![0u8; pw * ph];
            for row in 0..ph {
                let src = slice::from_raw_parts(ptr.add(row * row_stride), pw);
                out[row * pw..row * pw + pw].copy_from_slice(src);
            }
            out
        };
        let y = copy_plane(pic.y, pic.y_stride as usize, w, h);
        let u = copy_plane(pic.u, pic.uv_stride as usize, cw, ch);
        let v = copy_plane(pic.v, pic.uv_stride as usize, cw, ch);
        libwebp_sys::WebPPictureFree(&mut pic);
        DecodedYuv {
            width,
            height,
            y,
            u,
            v,
        }
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

/// Encodes an interleaved RGBA image to a lossy (VP8) WebP file with libwebp at `quality` (`0..=100`),
/// returning the file bytes. This drives the reverse-direction oracle: a real libwebp encoder's VP8
/// stream (with its own filter, segmentation, and probability choices) that gamut must decode.
///
/// `rgba` must be exactly `width * height * 4` bytes. Panics (these are tests) on a bad buffer size or
/// a libwebp error.
#[must_use]
pub fn libwebp_encode_lossy_rgba(rgba: &[u8], width: u32, height: u32, quality: f32) -> Vec<u8> {
    let expected = width as usize * height as usize * 4;
    assert_eq!(
        rgba.len(),
        expected,
        "RGBA buffer is not width*height*4 bytes"
    );

    let mut out_ptr: *mut u8 = std::ptr::null_mut();
    let stride = width as c_int * 4;
    // SAFETY: `rgba` is valid for `expected` bytes; `out_ptr` receives a libwebp-allocated buffer we
    // copy out and free below. The dimensions/stride describe `rgba` exactly.
    let size = unsafe {
        libwebp_sys::WebPEncodeRGBA(
            rgba.as_ptr(),
            width as c_int,
            height as c_int,
            stride,
            quality,
            &mut out_ptr,
        )
    };
    assert!(
        size != 0 && !out_ptr.is_null(),
        "libwebp lossy encode failed"
    );
    // SAFETY: libwebp guarantees `out_ptr` points to `size` valid bytes.
    let bytes = unsafe { slice::from_raw_parts(out_ptr, size) }.to_vec();
    // SAFETY: `out_ptr` was allocated by libwebp and must be released with WebPFree.
    unsafe { libwebp_sys::WebPFree(out_ptr.cast::<c_void>()) };
    bytes
}

/// libwebp lossy-encoder knobs the one-shot [`libwebp_encode_lossy_rgba`] (`WebPEncodeRGBA`) cannot
/// reach. Driving these through the advanced `WebPEncode` path lets the reverse-direction oracle
/// *force* decode features a real encoder can emit but cwebp's defaults rarely do — the simple loop
/// filter, a chosen segment count, low/high encoder effort — so gamut's decoder is pinned against the
/// full VP8 surface rather than whatever libwebp happens to pick.
///
/// Note: `WebPConfig.partitions` (token-partition count) is deliberately absent — libwebp's encoder
/// only range-checks it and always writes a single partition (`config_enc.c`), so it cannot source a
/// multi-partition stream. gamut's multi-partition *decode* is instead pinned in the forward
/// direction (gamut encodes 2/4/8 partitions → libwebp decodes them) in
/// `gamut_lossy_options_match_libwebp_bit_exact`.
#[derive(Clone, Copy, Debug)]
pub struct LibwebpLossyConfig {
    /// Quality factor, `0.0..=100.0`.
    pub quality: f32,
    /// Loop-filter type: `0` = simple, `1` = complex (normal).
    pub filter_type: i32,
    /// Number of segments, `1..=4`.
    pub segments: i32,
    /// Encoder method / effort, `0..=6` (deeper analysis ⇒ more probability updates, segmentation).
    pub method: i32,
    /// Deblocking filter strength, `0..=100` (`0` disables the loop filter).
    pub filter_strength: i32,
}

/// Appends libwebp's emitted output chunks into the `Vec<u8>` behind `picture.custom_ptr`. Matches the
/// `WebPWriterFunction` ABI so it can be installed as `picture.writer`.
extern "C" fn collect_writer(
    data: *const u8,
    data_size: usize,
    picture: *const libwebp_sys::WebPPicture,
) -> c_int {
    // SAFETY: `picture.custom_ptr` is the `&mut Vec<u8>` installed in `libwebp_encode_lossy_rgba_config`
    // below; `data` is valid for `data_size` bytes for the duration of this callback.
    unsafe {
        let out = &mut *((*picture).custom_ptr.cast::<Vec<u8>>());
        out.extend_from_slice(slice::from_raw_parts(data, data_size));
    }
    1
}

/// Encodes interleaved RGBA to a lossy WebP via libwebp's **advanced** `WebPEncode` API under an
/// explicit [`LibwebpLossyConfig`], returning the file bytes. Unlike [`libwebp_encode_lossy_rgba`],
/// this forces specific VP8 encoder features so the reverse-direction oracle can pin gamut's decoder
/// against the full surface a production encoder *can* emit. Panics (these are tests) on a bad buffer
/// size or a libwebp error.
#[must_use]
pub fn libwebp_encode_lossy_rgba_config(
    rgba: &[u8],
    width: u32,
    height: u32,
    cfg: &LibwebpLossyConfig,
) -> Vec<u8> {
    let expected = width as usize * height as usize * 4;
    assert_eq!(
        rgba.len(),
        expected,
        "RGBA buffer is not width*height*4 bytes"
    );

    let mut out: Vec<u8> = Vec::new();
    // SAFETY: each libwebp struct is zero-initialised then filled by its `*Init` function before use;
    // the picture borrows `rgba` (via the import copy) and writes through `collect_writer` into `out`
    // for the duration of `WebPEncode` only; every raw pointer below stays valid across the calls.
    unsafe {
        let mut config: libwebp_sys::WebPConfig = std::mem::zeroed();
        assert!(
            libwebp_sys::WebPConfigInit(&mut config) != 0,
            "WebPConfigInit failed (version mismatch?)"
        );
        config.quality = cfg.quality;
        config.filter_type = cfg.filter_type;
        config.segments = cfg.segments;
        config.method = cfg.method;
        config.filter_strength = cfg.filter_strength;
        assert!(
            libwebp_sys::WebPValidateConfig(&config) != 0,
            "WebPValidateConfig rejected {cfg:?}"
        );

        let mut pic: libwebp_sys::WebPPicture = std::mem::zeroed();
        assert!(
            libwebp_sys::WebPPictureInit(&mut pic) != 0,
            "WebPPictureInit failed (version mismatch?)"
        );
        pic.width = width as c_int;
        pic.height = height as c_int;
        // Imports into the ARGB buffer (sets use_argb=1); WebPEncode converts ARGB→YUV for lossy.
        assert!(
            libwebp_sys::WebPPictureImportRGBA(&mut pic, rgba.as_ptr(), width as c_int * 4) != 0,
            "WebPPictureImportRGBA failed"
        );
        pic.writer = Some(collect_writer);
        pic.custom_ptr = std::ptr::from_mut(&mut out).cast::<c_void>();

        let ok = libwebp_sys::WebPEncode(&config, &mut pic);
        libwebp_sys::WebPPictureFree(&mut pic);
        assert!(ok != 0, "WebPEncode failed for {cfg:?}");
    }
    out
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

/// Generates a deterministic, fully-opaque RGBA image with **photographic-like statistics**: smooth
/// low-frequency gradients and a coarse blob (large correlated regions, the bulk of a real photo)
/// overlaid with low-amplitude high-frequency detail and a few hard rectangle edges. This drives the
/// encoder down realistic residual/token/back-reference paths that the purely algebraic
/// [`pattern_rgba`] never reaches, while staying RNG-free so the corpus is reproducible and
/// version-controlled. `seed` varies the content. Alpha is held at 255 (see [`pattern_rgba`]).
#[must_use]
pub fn photo_like_rgba(width: u32, height: u32, seed: u32) -> Vec<u8> {
    let (w, h) = (i64::from(width).max(1), i64::from(height).max(1));
    // Small integer value-noise: hash (x, y, seed) into a high-frequency byte in `0..=255`.
    let hash = |x: i64, y: i64| -> i64 {
        let mut v = x.wrapping_mul(374_761_393)
            ^ y.wrapping_mul(668_265_263)
            ^ i64::from(seed).wrapping_mul(2_246_822_519);
        v = (v ^ (v >> 13)).wrapping_mul(1_274_126_177);
        (v ^ (v >> 16)) & 0xff
    };
    let clamp = |v: i64| v.clamp(0, 255) as u8;
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..h {
        for x in 0..w {
            // Low-frequency smooth base: per-channel ramps plus a shared diagonal so large regions
            // correlate (what spatial prediction and LZ77 exploit).
            let base_r = x * 200 / w + y * 30 / h;
            let base_g = y * 200 / h + (x + y) * 20 / (w + h);
            let base_b = (x + y) * 160 / (w + h) + 40;
            // Low-amplitude detail (sensor-noise / texture analogue) and a few hard luminance steps
            // (sharp edges exercise B_PRED mode selection and the loop filter).
            let detail = (hash(x, y) - 128) / 12;
            let edge = if (x * 3 / w) % 2 == 0 && (y * 3 / h) % 2 == 0 {
                40
            } else {
                0
            };
            rgba.push(clamp(base_r + detail + edge)); // R
            rgba.push(clamp(base_g + detail)); // G
            rgba.push(clamp(base_b - detail + edge)); // B
            rgba.push(0xff); // A (opaque)
        }
    }
    rgba
}
