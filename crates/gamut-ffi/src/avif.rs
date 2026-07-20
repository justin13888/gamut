//! AVIF seam handle: [`GamutAvifEncoder`] and its C entry points.
//!
//! The AVIF seam is the AV1 still-picture OBU stream (`AV1_CODEC_ID`, encode direction).
//! Backends are tried in push order; the built-in `gamut-av1` encoder is the implicit tail.
//!
//! There is deliberately no `gamut_avif_decoder_push_backend`: AVIF decode has no registry —
//! the Rust API threads a single caller-supplied `Av1StillDecoder` per call, and the planned
//! in-house decoder (issue #259) will make it optional. The decode direction joins the C
//! surface when that lands (see DESIGN.md).

use core::ffi::c_void;

use gamut_codec_abi::EncoderVTable;

use crate::seam::seam_handle;
use crate::status::GamutStatus;

/// Opaque handle over a `gamut::avif::AvifEncoder` and its backend registry.
pub struct GamutAvifEncoder {
    inner: gamut::avif::AvifEncoder,
}

seam_handle! {
    handle = GamutAvifEncoder,
    host = gamut::avif::AvifEncoder,
    new = gamut::avif::AvifEncoder::new(),
    seam = encoder,
    push = |host, backend| { host.push_backend(gamut::avif::AbiAv1StillEncoder::new(backend)); },
    fns = (gamut_avif_encoder_new, gamut_avif_encoder_free, gamut_avif_encoder_push_backend),
    tests = avif_encoder_tests,
}

/// Creates an AVIF encoder with default (lossless) settings and an empty backend registry.
///
/// Returns `NULL` only if construction panics. Free with `gamut_avif_encoder_free`.
#[unsafe(no_mangle)]
pub extern "C" fn gamut_avif_encoder_new() -> *mut GamutAvifEncoder {
    GamutAvifEncoder::ffi_new()
}

/// Frees an encoder created by `gamut_avif_encoder_new`, running each adopted backend's
/// `destroy` exactly once. No-op on `NULL`.
///
/// # Safety
///
/// `encoder` is `NULL` or a pointer returned by `gamut_avif_encoder_new` that has not already
/// been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gamut_avif_encoder_free(encoder: *mut GamutAvifEncoder) {
    unsafe { GamutAvifEncoder::ffi_free(encoder) }
}

/// Pushes an AV1 still-picture encode backend; backends are tried in push order, before the
/// built-in `gamut-av1` encoder.
///
/// Returns `GAMUT_OK` when the handle adopts `ctx` (its `destroy` then runs exactly once, at
/// free), `GAMUT_STATUS_NULL_ARGUMENT` on a `NULL` encoder or vtable, or
/// `GAMUT_STATUS_ABI_MISMATCH` when `vtable->abi_version != GAMUT_CODEC_ABI_VERSION`; on any
/// non-OK status the caller keeps ownership of `ctx` and no callback has run.
///
/// # Safety
///
/// `encoder` as in `gamut_avif_encoder_free`; a non-`NULL` `vtable` points to a table that
/// stays valid for the encoder's lifetime. Calling this asserts the `(vtable, ctx)` backend
/// may be used from any thread (the registry requires `Send`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gamut_avif_encoder_push_backend(
    encoder: *mut GamutAvifEncoder,
    vtable: *const EncoderVTable,
    ctx: *mut c_void,
) -> GamutStatus {
    unsafe { GamutAvifEncoder::ffi_push_backend(encoder, vtable, ctx) }
}
