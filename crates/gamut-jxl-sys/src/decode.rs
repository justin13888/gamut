//! Decoder API subset of libjxl v0.12.0 (`jxl/decode.h`).
//!
//! [`gamut-jxl`](https://crates.io/crates/gamut-jxl) decodes with the pure-Rust `jxl` crate; this
//! decode surface exists solely as its **differential-test oracle** — the reference decoder that
//! gamut-jxl's tests cross-check the pure-Rust decoder against. The same static libjxl archive
//! contains both the encoder and decoder, so exposing this adds no extra native build.
//!
//! Declarations only. Function names keep libjxl's exact spelling, hence the module-level
//! `non_snake_case` allow.
#![allow(non_snake_case)]

use core::ffi::{c_int, c_void};

use crate::types::{JxlBasicInfo, JxlMemoryManager, JxlPixelFormat};

/// Opaque decoder instance (`JxlDecoder`). Created by [`JxlDecoderCreate`]
/// and destroyed by [`JxlDecoderDestroy`].
#[repr(C)]
pub struct JxlDecoder {
    _private: [u8; 0],
}

/// Result of [`JxlSignatureCheck`] (`JxlSignature`, `jxl/decode.h`).
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JxlSignature(pub c_int);

impl JxlSignature {
    /// Not enough bytes were passed to determine the signature (`JXL_SIG_NOT_ENOUGH_BYTES`).
    pub const NOT_ENOUGH_BYTES: Self = Self(0);
    /// No valid JPEG XL header was found (`JXL_SIG_INVALID`).
    pub const INVALID: Self = Self(1);
    /// A valid naked codestream signature was found (`JXL_SIG_CODESTREAM`).
    pub const CODESTREAM: Self = Self(2);
    /// A valid container (BMFF) signature was found (`JXL_SIG_CONTAINER`).
    pub const CONTAINER: Self = Self(3);
}

/// Return value for [`JxlDecoderProcessInput`] and other decoder functions, and the identifier of
/// subscribable events (`JxlDecoderStatus`, `jxl/decode.h`).
///
/// Only the subset the oracle uses is declared. The event values are a bit set as used with
/// [`JxlDecoderSubscribeEvents`]; the underlying C enum is `int`.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JxlDecoderStatus(pub c_int);

impl JxlDecoderStatus {
    /// Function call finished successfully, or decoding is finished (`JXL_DEC_SUCCESS`).
    pub const SUCCESS: Self = Self(0);
    /// An error occurred, e.g. invalid input or out of memory (`JXL_DEC_ERROR`).
    pub const ERROR: Self = Self(1);
    /// The decoder needs more input bytes to continue (`JXL_DEC_NEED_MORE_INPUT`).
    pub const NEED_MORE_INPUT: Self = Self(2);
    /// The decoder requests an output buffer for the full-resolution image
    /// (`JXL_DEC_NEED_IMAGE_OUT_BUFFER`).
    pub const NEED_IMAGE_OUT_BUFFER: Self = Self(5);
    /// Basic information (dimensions, extra channels) is available (`JXL_DEC_BASIC_INFO`).
    pub const BASIC_INFO: Self = Self(0x40);
    /// A full frame has been decoded (`JXL_DEC_FULL_IMAGE`).
    pub const FULL_IMAGE: Self = Self(0x1000);
}

unsafe extern "C" {
    /// Returns the decoder library version as `MAJOR*1000000 + MINOR*1000 + PATCH`
    /// (`JxlDecoderVersion`).
    ///
    /// # Safety
    ///
    /// Always safe to call: it takes no arguments and dereferences no pointers.
    pub fn JxlDecoderVersion() -> u32;

    /// Checks the JPEG XL signature of the first bytes of a stream (`JxlSignatureCheck`).
    ///
    /// # Safety
    ///
    /// `buf` must point to at least `len` readable bytes.
    pub fn JxlSignatureCheck(buf: *const u8, len: usize) -> JxlSignature;

    /// Creates and initializes a [`JxlDecoder`], or returns null on failure (`JxlDecoderCreate`).
    ///
    /// # Safety
    ///
    /// `memory_manager` must be null or point to a valid [`JxlMemoryManager`] that outlives the
    /// decoder. The returned pointer must be freed with [`JxlDecoderDestroy`].
    pub fn JxlDecoderCreate(memory_manager: *const JxlMemoryManager) -> *mut JxlDecoder;

    /// Re-initializes a decoder for reuse, keeping its memory manager (`JxlDecoderReset`).
    ///
    /// # Safety
    ///
    /// `dec` must be a valid pointer from [`JxlDecoderCreate`] that has not been destroyed.
    pub fn JxlDecoderReset(dec: *mut JxlDecoder);

    /// Deinitializes and frees a decoder (`JxlDecoderDestroy`).
    ///
    /// # Safety
    ///
    /// `dec` must be null or a valid pointer from [`JxlDecoderCreate`] that has not already been
    /// destroyed. The pointer is dangling afterwards.
    pub fn JxlDecoderDestroy(dec: *mut JxlDecoder);

    /// Subscribes the decoder to a bit set of events (e.g.
    /// [`JxlDecoderStatus::BASIC_INFO`] | [`JxlDecoderStatus::FULL_IMAGE`])
    /// (`JxlDecoderSubscribeEvents`).
    ///
    /// # Safety
    ///
    /// `dec` must be a valid decoder that has not yet started processing input.
    pub fn JxlDecoderSubscribeEvents(
        dec: *mut JxlDecoder,
        events_wanted: c_int,
    ) -> JxlDecoderStatus;

    /// Sets the next input buffer for the decoder (`JxlDecoderSetInput`). The buffer must remain
    /// valid and unchanged until [`JxlDecoderReleaseInput`] or [`JxlDecoderDestroy`].
    ///
    /// # Safety
    ///
    /// `dec` must be valid. `data` must point to at least `size` readable bytes that stay alive and
    /// unmodified until released.
    pub fn JxlDecoderSetInput(
        dec: *mut JxlDecoder,
        data: *const u8,
        size: usize,
    ) -> JxlDecoderStatus;

    /// Releases the input buffer set with [`JxlDecoderSetInput`], returning the number of unprocessed
    /// bytes (`JxlDecoderReleaseInput`).
    ///
    /// # Safety
    ///
    /// `dec` must be a valid decoder.
    pub fn JxlDecoderReleaseInput(dec: *mut JxlDecoder) -> usize;

    /// Marks the current input as final, so the decoder knows no more bytes will follow
    /// (`JxlDecoderCloseInput`).
    ///
    /// # Safety
    ///
    /// `dec` must be a valid decoder.
    pub fn JxlDecoderCloseInput(dec: *mut JxlDecoder);

    /// Decodes the available input up to the next subscribed event (`JxlDecoderProcessInput`).
    ///
    /// # Safety
    ///
    /// `dec` must be a valid decoder with input set via [`JxlDecoderSetInput`].
    pub fn JxlDecoderProcessInput(dec: *mut JxlDecoder) -> JxlDecoderStatus;

    /// Copies the basic image information into `info`, if available (`JxlDecoderGetBasicInfo`).
    ///
    /// # Safety
    ///
    /// `dec` must be valid. `info` must be null or point to writable storage for a [`JxlBasicInfo`].
    pub fn JxlDecoderGetBasicInfo(
        dec: *const JxlDecoder,
        info: *mut JxlBasicInfo,
    ) -> JxlDecoderStatus;

    /// Writes the minimum output buffer size, in bytes, for the given pixel format into `*size`
    /// (`JxlDecoderImageOutBufferSize`).
    ///
    /// # Safety
    ///
    /// `dec` must be valid, `format` must point to a valid [`JxlPixelFormat`], and `size` to a
    /// writable `usize`.
    pub fn JxlDecoderImageOutBufferSize(
        dec: *const JxlDecoder,
        format: *const JxlPixelFormat,
        size: *mut usize,
    ) -> JxlDecoderStatus;

    /// Sets the output buffer the decoder writes full-resolution pixels into
    /// (`JxlDecoderSetImageOutBuffer`). The buffer must be at least
    /// [`JxlDecoderImageOutBufferSize`] bytes.
    ///
    /// # Safety
    ///
    /// `dec` must be valid, `format` must point to a valid [`JxlPixelFormat`], and `buffer` must
    /// point to at least `size` writable bytes that stay alive until decoding of the frame completes.
    pub fn JxlDecoderSetImageOutBuffer(
        dec: *mut JxlDecoder,
        format: *const JxlPixelFormat,
        buffer: *mut c_void,
        size: usize,
    ) -> JxlDecoderStatus;
}
