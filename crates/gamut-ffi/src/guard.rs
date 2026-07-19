//! Panic containment: every `extern "C"` entry point runs its body under `catch_unwind`, so a
//! Rust panic becomes [`GAMUT_STATUS_PANIC`](crate::GAMUT_STATUS_PANIC) (or `NULL`, or a
//! no-op) instead of unwinding across the C boundary. The policy lives here once; the
//! `seam_handle!` internals go through these helpers, so an entry point cannot forget it.

use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::status::{GAMUT_STATUS_PANIC, GamutStatus};

/// Runs `f`, mapping a panic to [`GAMUT_STATUS_PANIC`].
pub(crate) fn status(f: impl FnOnce() -> GamutStatus) -> GamutStatus {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(GAMUT_STATUS_PANIC)
}

/// Runs `f`, mapping a panic to a null pointer.
pub(crate) fn ptr<T>(f: impl FnOnce() -> *mut T) -> *mut T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(std::ptr::null_mut())
}

/// Runs `f`, swallowing a panic — for `_free` entry points, which return nothing.
pub(crate) fn unit(f: impl FnOnce()) {
    let _ = catch_unwind(AssertUnwindSafe(f));
}
