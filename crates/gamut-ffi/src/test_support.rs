//! Stub vtables for the `seam_handle!`-stamped per-handle contract tests: current-ABI tables
//! whose only behavior is counting `destroy` calls through their `ctx`.

use core::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use gamut_codec_abi::{ABI_VERSION, DecoderVTable, EncoderVTable};

/// Increments the `AtomicUsize` behind `ctx`.
unsafe extern "C" fn count_destroy(ctx: *mut c_void) {
    // SAFETY: the tests pass a pointer to a live AtomicUsize as `ctx`, which outlives the
    // handle that owns the backend.
    unsafe { &*ctx.cast_const().cast::<AtomicUsize>() }.fetch_add(1, Ordering::SeqCst);
}

/// A current-ABI decoder vtable that only counts `destroy` calls.
pub(crate) fn decoder_vtable() -> DecoderVTable {
    DecoderVTable {
        abi_version: ABI_VERSION,
        supports: None,
        decode: None,
        destroy: Some(count_destroy),
    }
}

/// A current-ABI encoder vtable that only counts `destroy` calls.
pub(crate) fn encoder_vtable() -> EncoderVTable {
    EncoderVTable {
        abi_version: ABI_VERSION,
        supports: None,
        encode: None,
        destroy: Some(count_destroy),
    }
}
