//! HEIC seam handle: [`GamutHeicDecoder`] and its C entry points.
//!
//! The HEIC seam is the HEVC intra codestream (`hvc1`, decode direction — gamut ships **no**
//! software HEVC tail, issue #18, so an empty registry decodes nothing). Unlike the other
//! formats, the handle stores the pushed [`ForeignDecoder`]s raw, in push order, rather than
//! pre-built `AbiHevcDecoder` adapters: an `hvcC` record carries no picture size, so the
//! adapter needs each item's `ispe` dimensions and can only be built per decoded item. The
//! consumer entry points of issue #242 lend each stored backend to a fresh per-item adapter
//! (`AbiHevcDecoder::new(&mut foreign, dims)` via `gamut-codec-abi`'s `&mut` blanket impl) —
//! the composition pinned by the const block below and by gamut-heic's
//! `abi_borrowed_backend` test.

use core::ffi::c_void;

use gamut_codec_abi::DecoderVTable;
use gamut_codec_abi::bridge::ForeignDecoder;

use crate::seam::seam_handle;
use crate::status::GamutStatus;

/// Opaque handle over the HEIC HEVC-decode backend registry (push order).
pub struct GamutHeicDecoder {
    inner: Vec<ForeignDecoder>,
}

// Living-shim tie to the Rust seam this handle feeds: a stored ForeignDecoder must remain
// lendable to a per-item AbiHevcDecoder. If gamut-heic's adapter contract drifts, this fails
// to compile here rather than at issue #242's decode entry point.
const _: () = {
    fn _per_item_adapter(
        backend: &mut ForeignDecoder,
        dimensions: gamut::core::Dimensions,
    ) -> impl gamut::heic::HevcDecoder + '_ {
        gamut::heic::AbiHevcDecoder::new(backend, dimensions)
    }
};

seam_handle! {
    handle = GamutHeicDecoder,
    host = Vec<ForeignDecoder>,
    new = Vec::new(),
    seam = decoder,
    push = |host, backend| { host.push(backend); },
    fns = (gamut_heic_decoder_new, gamut_heic_decoder_free, gamut_heic_decoder_push_backend),
    tests = heic_decoder_tests,
}

/// Creates a HEIC decoder with an empty backend registry.
///
/// gamut ships no built-in HEVC decoder: until a backend is pushed, the handle can parse
/// containers (issue #242) but decodes nothing. Returns `NULL` only if construction panics.
/// Free with `gamut_heic_decoder_free`.
#[unsafe(no_mangle)]
pub extern "C" fn gamut_heic_decoder_new() -> *mut GamutHeicDecoder {
    GamutHeicDecoder::ffi_new()
}

/// Frees a decoder created by `gamut_heic_decoder_new`, running each adopted backend's
/// `destroy` exactly once. No-op on `NULL`.
///
/// # Safety
///
/// `decoder` is `NULL` or a pointer returned by `gamut_heic_decoder_new` that has not already
/// been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gamut_heic_decoder_free(decoder: *mut GamutHeicDecoder) {
    unsafe { GamutHeicDecoder::ffi_free(decoder) }
}

/// Pushes an HEVC intra-decode backend; backends are consulted in push order (there is no
/// built-in tail).
///
/// Returns `GAMUT_OK` when the handle adopts `ctx` (its `destroy` then runs exactly once, at
/// free), `GAMUT_STATUS_NULL_ARGUMENT` on a `NULL` decoder or vtable, or
/// `GAMUT_STATUS_ABI_MISMATCH` when `vtable->abi_version != GAMUT_CODEC_ABI_VERSION`; on any
/// non-OK status the caller keeps ownership of `ctx` and no callback has run.
///
/// # Safety
///
/// `decoder` as in `gamut_heic_decoder_free`; a non-`NULL` `vtable` points to a table that
/// stays valid for the decoder's lifetime. Calling this asserts the `(vtable, ctx)` backend
/// may be used from any thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gamut_heic_decoder_push_backend(
    decoder: *mut GamutHeicDecoder,
    vtable: *const DecoderVTable,
    ctx: *mut c_void,
) -> GamutStatus {
    unsafe { GamutHeicDecoder::ffi_push_backend(decoder, vtable, ctx) }
}
