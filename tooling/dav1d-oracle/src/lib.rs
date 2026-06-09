//! Dev-only differential oracle around a vendored, statically-linked **libdav1d**.
//!
//! gamut's AV1 still-image encoder maintains a reconstruction buffer that must equal, sample for
//! sample, what a conformant decoder produces. This crate decodes the encoder's raw AV1 OBU stream
//! with the real dav1d decoder so the cross-check tests can assert byte-exact equality — without
//! depending on a `dav1d` binary being installed on the host. The C library is built from the
//! `third_party/dav1d` git submodule by `build.rs`; see that file for the build wiring.
//!
//! All `unsafe` FFI is confined here behind a single safe entry point, [`decode_obu`].

#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case)]

use std::os::raw::c_int;
use std::ptr;

mod sys {
    #![allow(dead_code)]
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

/// A decoded 8-bit planar picture: one tightly packed `width * height` plane per component.
///
/// For 4:2:0 / 4:2:2 content the chroma planes are subsampled, so `planes[1]` / `planes[2]` are
/// smaller than `planes[0]`; for the 4:4:4 stills gamut emits, all three are `width * height`.
pub struct DecodedPicture {
    /// Luma width in pixels.
    pub width: u32,
    /// Luma height in pixels.
    pub height: u32,
    /// `[Y, U, V]` planes, each in raster order with no row padding.
    pub planes: [Vec<u8>; 3],
}

/// `DAV1D_ERR(EAGAIN)`: the decoder's "feed me more data / drain a picture first" sentinel. dav1d
/// negates the POSIX `EAGAIN`, whose value differs between Linux (11) and the BSD/macOS family (35).
fn err_again() -> c_int {
    let eagain = if cfg!(target_os = "linux") { 11 } else { 35 };
    -eagain
}

/// Decodes a single still frame from a low-overhead (Section 5) AV1 OBU stream.
///
/// `obus` must contain a complete temporal unit (a temporal-delimiter OBU followed by the
/// sequence/frame OBUs, each carrying its own size field). Returns the first decoded picture.
///
/// # Errors
///
/// Returns a message if dav1d cannot be initialized, the stream produces no picture, or the
/// decoded picture is not 8-bit (gamut only emits 8-bit stills).
pub fn decode_obu(obus: &[u8]) -> Result<DecodedPicture, String> {
    // SAFETY: every pointer handed to dav1d below is either a stack value we own for the duration
    // of the call or a buffer dav1d itself allocated; we pair every successful `*_create`/picture
    // acquisition with the matching `*_unref`/`close` before returning, on both the ok and err
    // paths.
    unsafe { decode_obu_inner(obus) }
}

unsafe fn decode_obu_inner(obus: &[u8]) -> Result<DecodedPicture, String> {
    // SAFETY (whole body): FFI calls and pointer copies into dav1d-owned buffers; every acquired
    // resource is released before any return (see the matching `*_unref`/`close` at the end).
    unsafe {
        let mut settings: sys::Dav1dSettings = std::mem::zeroed();
        sys::dav1d_default_settings(&mut settings);
        // Single-threaded, frame-latency 1: deterministic and lets one send → one get drain a still.
        settings.n_threads = 1;
        settings.max_frame_delay = 1;

        let mut ctx: *mut sys::Dav1dContext = ptr::null_mut();
        if sys::dav1d_open(&mut ctx, &settings) != 0 {
            return Err("dav1d_open failed".into());
        }

        let mut data: sys::Dav1dData = std::mem::zeroed();
        let buf = sys::dav1d_data_create(&mut data, obus.len());
        if buf.is_null() {
            sys::dav1d_close(&mut ctx);
            return Err("dav1d_data_create failed".into());
        }
        ptr::copy_nonoverlapping(obus.as_ptr(), buf, obus.len());

        let mut pic: sys::Dav1dPicture = std::mem::zeroed();
        let got_picture = loop {
            if data.sz > 0 {
                let r = sys::dav1d_send_data(ctx, &mut data);
                if r < 0 && r != err_again() {
                    break Err(format!("dav1d_send_data failed: {r}"));
                }
            }
            let r = sys::dav1d_get_picture(ctx, &mut pic);
            if r == 0 {
                break Ok(());
            }
            if r != err_again() {
                break Err(format!("dav1d_get_picture failed: {r}"));
            }
            // EAGAIN: only meaningful while there is still data left to feed.
            if data.sz == 0 {
                break Err("dav1d produced no picture from the stream".into());
            }
        };

        let result = got_picture.and_then(|()| extract(&pic));

        if !pic.data[0].is_null() {
            sys::dav1d_picture_unref(&mut pic);
        }
        sys::dav1d_data_unref(&mut data);
        sys::dav1d_close(&mut ctx);
        result
    }
}

/// Copies the three planes out of a decoded dav1d picture into owned, unpadded buffers.
unsafe fn extract(pic: &sys::Dav1dPicture) -> Result<DecodedPicture, String> {
    if pic.p.bpc != 8 {
        return Err(format!("expected 8-bit picture, got {} bpc", pic.p.bpc));
    }
    let w = pic.p.w as usize;
    let h = pic.p.h as usize;
    let (cw, ch) = chroma_dims(pic.p.layout, w, h);

    // SAFETY: `pic` is a live dav1d picture; its `data`/`stride` describe planes of at least the
    // dimensions reported in `pic.p` for the given layout.
    unsafe {
        let y = copy_plane(pic.data[0].cast::<u8>(), pic.stride[0], w, h);
        let u = copy_plane(pic.data[1].cast::<u8>(), pic.stride[1], cw, ch);
        let v = copy_plane(pic.data[2].cast::<u8>(), pic.stride[1], cw, ch);
        Ok(DecodedPicture {
            width: w as u32,
            height: h as u32,
            planes: [y, u, v],
        })
    }
}

/// Chroma plane dimensions for a given pixel layout (luma `w`×`h`).
fn chroma_dims(layout: sys::Dav1dPixelLayout, w: usize, h: usize) -> (usize, usize) {
    match layout {
        sys::DAV1D_PIXEL_LAYOUT_I420 => (w.div_ceil(2), h.div_ceil(2)),
        sys::DAV1D_PIXEL_LAYOUT_I422 => (w.div_ceil(2), h),
        // I400 has no chroma; treat as zero-sized. I444 keeps full resolution.
        sys::DAV1D_PIXEL_LAYOUT_I400 => (0, 0),
        _ => (w, h),
    }
}

/// Copies a `w`×`h` 8-bit plane from a strided dav1d buffer into a tightly packed `Vec`.
unsafe fn copy_plane(base: *const u8, stride: isize, w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w * h];
    // SAFETY: caller guarantees `base` addresses `h` rows of at least `w` bytes spaced `stride`
    // apart; `out` is exactly `w * h` bytes, so each row copy stays in bounds of both buffers.
    unsafe {
        for row in 0..h {
            let src = base.offset(stride * row as isize);
            ptr::copy_nonoverlapping(src, out.as_mut_ptr().add(row * w), w);
        }
    }
    out
}
