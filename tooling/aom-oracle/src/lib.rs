//! Dev-only differential oracle around a vendored, statically-linked **libaom** — the AV1
//! **reference codec** (AOMedia's reference encoder *and* decoder).
//!
//! gamut's AV1 still-image encoder maintains a reconstruction buffer that must equal, sample
//! for sample, what a conformant decoder produces. [`decode_av1`] runs the encoder's raw AV1
//! stream through libaom's reference decoder so the cross-check tests can assert byte-exact
//! equality against the most authoritative implementation — without depending on a system
//! `aom` install.
//!
//! [`encode_still_intra`] is the mirror direction: libaom's reference **encoder** produces a
//! conformant AV1 still bitstream. gamut has no AV1 decoder yet (it is encoder-first), so this
//! is the golden oracle awaiting that future decoder — a reference bitstream source to decode
//! and check against. It is exercised today by this crate's own lossless round-trip self-test.
//!
//! The C library is built from the `third_party/aom` git submodule by `build.rs`; see that
//! file for the build wiring. All `unsafe` FFI is confined here behind two safe entry points.

#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case)]

use std::ffi::CStr;
use std::os::raw::c_int;
use std::ptr;

mod sys {
    #![allow(dead_code)]
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

/// A decoded planar picture: one tightly packed `width * height` plane per component, with each
/// sample widened to `u16` (8-bit samples occupy `0..=255`, 10-/12-bit use the wider range).
///
/// For 4:2:0 / 4:2:2 content the chroma planes are subsampled, so `planes[1]` / `planes[2]` are
/// smaller than `planes[0]`; for the 4:4:4 stills gamut emits, all three are `width * height`.
pub struct DecodedPicture {
    /// Luma width in pixels.
    pub width: u32,
    /// Luma height in pixels.
    pub height: u32,
    /// Bits per component (8, 10, or 12).
    pub bit_depth: u8,
    /// `[Y, U, V]` planes, each in raster order with no row padding; samples widened to `u16`.
    pub planes: [Vec<u16>; 3],
}

/// Decodes a single still frame from an AV1 temporal unit with libaom's reference decoder.
///
/// `data` must contain a complete temporal unit (a temporal-delimiter OBU followed by the
/// sequence/frame OBUs, each carrying its own size field). Returns the first decoded picture.
///
/// # Errors
///
/// Returns a message if libaom cannot be initialized, the stream produces no picture, or the
/// decoded picture is not 8/10/12-bit (the bit depths gamut emits).
pub fn decode_av1(data: &[u8]) -> Result<DecodedPicture, String> {
    // SAFETY: the context is a stack value we own for the call; every successful `*_init` is
    // paired with `aom_codec_destroy` before returning on both the ok and err paths, and the
    // `aom_image_t` we read is owned by (and outlives our read within) that context.
    unsafe { decode_inner(data) }
}

unsafe fn decode_inner(data: &[u8]) -> Result<DecodedPicture, String> {
    // SAFETY (whole body): FFI calls into libaom; the context is destroyed before every return.
    unsafe {
        let iface = sys::aom_codec_av1_dx();
        let mut ctx: sys::aom_codec_ctx_t = std::mem::zeroed();
        let r = sys::aomshim_dec_init(&mut ctx, iface);
        if r != sys::AOM_CODEC_OK {
            return Err(format!("aom_codec_dec_init failed: {}", err_str(r)));
        }

        let r = sys::aom_codec_decode(&mut ctx, data.as_ptr(), data.len(), ptr::null_mut());
        let result = if r != sys::AOM_CODEC_OK {
            Err(format!("aom_codec_decode failed: {}", err_str(r)))
        } else {
            let mut iter: sys::aom_codec_iter_t = ptr::null();
            let img = sys::aom_codec_get_frame(&mut ctx, &mut iter);
            if img.is_null() {
                Err("aom produced no picture from the stream".to_string())
            } else {
                extract(&*img)
            }
        };

        sys::aom_codec_destroy(&mut ctx);
        result
    }
}

/// Copies the (up to) three planes out of a decoded libaom picture into owned, unpadded buffers.
unsafe fn extract(img: &sys::aom_image_t) -> Result<DecodedPicture, String> {
    let bpc = img.bit_depth as u8;
    if !matches!(bpc, 8 | 10 | 12) {
        return Err(format!("unexpected bit depth: {bpc} bpc"));
    }
    let w = img.d_w as usize;
    let h = img.d_h as usize;
    // Chroma plane dimensions follow the reported subsampling shifts (0/0 for the 4:4:4 stills
    // gamut emits, 1/1 for 4:2:0, 1/0 for 4:2:2). A null plane (monochrome) yields an empty Vec.
    let cw = w.div_ceil(1usize << img.x_chroma_shift);
    let ch = h.div_ceil(1usize << img.y_chroma_shift);

    // SAFETY: `img` is a live libaom picture; its `planes`/`stride` describe planes of at least
    // the dimensions reported in the image header for its format.
    unsafe {
        let y = copy_plane(img.planes[0], img.stride[0], w, h, bpc);
        let u = copy_plane(img.planes[1], img.stride[1], cw, ch, bpc);
        let v = copy_plane(img.planes[2], img.stride[2], cw, ch, bpc);
        Ok(DecodedPicture {
            width: w as u32,
            height: h as u32,
            bit_depth: bpc,
            planes: [y, u, v],
        })
    }
}

/// Copies a `w`×`h` plane from a strided libaom buffer into a tightly packed `u16` `Vec`. `bpc`
/// is the bit depth: at 8 the source samples are bytes (widened to `u16`); at 10/12 they are
/// native-endian `u16` and `byte_stride` is in bytes. A null base or zero-sized plane yields an
/// empty `Vec`.
unsafe fn copy_plane(base: *const u8, byte_stride: c_int, w: usize, h: usize, bpc: u8) -> Vec<u16> {
    if base.is_null() || w == 0 || h == 0 {
        return Vec::new();
    }
    let byte_stride = byte_stride as isize;
    let mut out = vec![0u16; w * h];
    // SAFETY: caller guarantees `base` addresses `h` rows of at least `w` samples spaced
    // `byte_stride` bytes apart; each read stays within row `row`'s `w` samples and `out` is
    // exactly `w * h` elements.
    unsafe {
        for row in 0..h {
            let row_base = base.offset(byte_stride * row as isize);
            for col in 0..w {
                out[row * w + col] = if bpc == 8 {
                    u16::from(*row_base.add(col))
                } else {
                    *row_base.cast::<u16>().add(col)
                };
            }
        }
    }
    out
}

/// Reference-encodes an 8-bit 4:4:4 still with libaom's all-intra encoder, returning the AV1
/// temporal unit (OBUs, each carrying its own size field — the low-overhead stream a decoder
/// consumes directly).
///
/// The three planes are Y, U, V in raster order, each exactly `width * height` bytes (full
/// resolution — 4:4:4). `qindex` selects the quantizer: `0` requests a bit-exact **lossless**
/// encode (the mode the round-trip self-test relies on); `1..=255` is mapped monotonically onto
/// libaom's constant-quality level (`0..=63`), coarser for higher `qindex`, matching gamut's
/// `base_q_idx` sense.
///
/// This is the reference-encoder half of the oracle, provided for the future gamut AV1 decoder;
/// it does not attempt to reproduce gamut's exact bitstream choices.
///
/// # Errors
///
/// Returns a message if the plane lengths are inconsistent with `width * height`, or if libaom
/// fails to configure, initialize, or encode.
pub fn encode_still_intra(
    width: u32,
    height: u32,
    y: &[u8],
    u: &[u8],
    v: &[u8],
    qindex: u8,
) -> Result<Vec<u8>, String> {
    let n = (width as usize)
        .checked_mul(height as usize)
        .ok_or("width * height overflows")?;
    if y.len() != n || u.len() != n || v.len() != n {
        return Err(format!(
            "each plane must be width*height = {n} bytes (got {}, {}, {})",
            y.len(),
            u.len(),
            v.len()
        ));
    }
    if width == 0 || height == 0 {
        return Err("width and height must be non-zero".to_string());
    }
    // SAFETY: every libaom resource acquired below (image, encoder context) is freed before
    // each return path.
    unsafe { encode_inner(width, height, y, u, v, qindex) }
}

unsafe fn encode_inner(
    width: u32,
    height: u32,
    y: &[u8],
    u: &[u8],
    v: &[u8],
    qindex: u8,
) -> Result<Vec<u8>, String> {
    // SAFETY (whole body): FFI into libaom; the image and context are freed before every return.
    unsafe {
        let iface = sys::aom_codec_av1_cx();

        let mut cfg: sys::aom_codec_enc_cfg_t = std::mem::zeroed();
        let r = sys::aom_codec_enc_config_default(iface, &mut cfg, sys::AOM_USAGE_ALL_INTRA);
        if r != sys::AOM_CODEC_OK {
            return Err(format!(
                "aom_codec_enc_config_default failed: {}",
                err_str(r)
            ));
        }
        cfg.g_w = width;
        cfg.g_h = height;
        // 4:4:4 8-bit requires seq_profile 1 (High); profile 0 rejects 4:4:4.
        cfg.g_profile = 1;
        cfg.g_bit_depth = sys::AOM_BITS_8;
        cfg.g_input_bit_depth = 8;
        cfg.g_threads = 1;
        cfg.rc_end_usage = sys::AOM_Q;

        let mut ctx: sys::aom_codec_ctx_t = std::mem::zeroed();
        let r = sys::aomshim_enc_init(&mut ctx, iface, &cfg);
        if r != sys::AOM_CODEC_OK {
            return Err(format!("aom_codec_enc_init failed: {}", err_str(r)));
        }

        // Fastest encode: this is an oracle, not a quality benchmark.
        control(&mut ctx, sys::AOME_SET_CPUUSED, 6);
        if qindex == 0 {
            control(&mut ctx, sys::AV1E_SET_LOSSLESS, 1);
        } else {
            // Map base_q_idx sense (0..255, coarser as it grows) onto CQ level (0..63).
            let cq = (i32::from(qindex) * 63 / 255).clamp(0, 63);
            control(&mut ctx, sys::AOME_SET_CQ_LEVEL, cq);
        }

        let mut img: sys::aom_image_t = std::mem::zeroed();
        if sys::aom_img_alloc(&mut img, sys::AOM_IMG_FMT_I444, width, height, 1).is_null() {
            sys::aom_codec_destroy(&mut ctx);
            return Err("aom_img_alloc failed".to_string());
        }
        copy_into_plane(img.planes[0], img.stride[0], y, width, height);
        copy_into_plane(img.planes[1], img.stride[1], u, width, height);
        copy_into_plane(img.planes[2], img.stride[2], v, width, height);

        // Encode the frame, then flush by passing a null image; drain packets after each call.
        let mut out = Vec::new();
        let mut encode_err = None;
        for frame in [ptr::addr_of!(img), ptr::null()] {
            let r = sys::aom_codec_encode(&mut ctx, frame, 0, 1, 0);
            if r != sys::AOM_CODEC_OK {
                encode_err = Some(format!("aom_codec_encode failed: {}", err_str(r)));
                break;
            }
            let mut iter: sys::aom_codec_iter_t = ptr::null();
            loop {
                let pkt = sys::aom_codec_get_cx_data(&mut ctx, &mut iter);
                if pkt.is_null() {
                    break;
                }
                if (*pkt).kind == sys::AOM_CODEC_CX_FRAME_PKT {
                    let f = &(*pkt).data.frame;
                    out.extend_from_slice(std::slice::from_raw_parts(f.buf.cast::<u8>(), f.sz));
                }
            }
        }

        sys::aom_img_free(&mut img);
        sys::aom_codec_destroy(&mut ctx);

        match encode_err {
            Some(e) => Err(e),
            None if out.is_empty() => Err("aom produced no compressed data".to_string()),
            None => Ok(out),
        }
    }
}

/// Sets an integer-valued encoder control, ignoring the return code (the IDs used here are all
/// valid for the AV1 encoder; a failure would surface as a downstream encode error).
unsafe fn control(ctx: *mut sys::aom_codec_ctx_t, id: sys::aome_enc_control_id, value: c_int) {
    // SAFETY: `ctx` is a live encoder context; `aom_codec_control` is variadic and these control
    // IDs each take a single `int` argument.
    unsafe {
        sys::aom_codec_control(ctx, id as c_int, value);
    }
}

/// Copies a full-resolution 8-bit plane into a strided libaom image buffer.
unsafe fn copy_into_plane(dst: *mut u8, byte_stride: c_int, src: &[u8], w: u32, h: u32) {
    let (w, h, stride) = (w as usize, h as usize, byte_stride as usize);
    // SAFETY: `dst` addresses `h` rows spaced `stride` bytes apart with room for `w` samples
    // each (libaom allocated it for a `w`×`h` plane); `src` is exactly `w * h` bytes.
    unsafe {
        for row in 0..h {
            ptr::copy_nonoverlapping(src[row * w..].as_ptr(), dst.add(row * stride), w);
        }
    }
}

/// Maps a libaom error code to its human-readable string.
fn err_str(err: sys::aom_codec_err_t) -> String {
    // SAFETY: `aom_codec_err_to_string` maps a code to a static C string and is always safe.
    let p = unsafe { sys::aom_codec_err_to_string(err) };
    if p.is_null() {
        format!("aom error {err}")
    } else {
        // SAFETY: `p` is a non-null static NUL-terminated string owned by libaom.
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}
