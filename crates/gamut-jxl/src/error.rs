//! Mapping from libjxl encoder status/error codes to the shared [`gamut_core::Error`] surface.
//!
//! Kept out of the `unsafe` [`crate::ffi`] module because it is pure value translation: it matches on
//! the plain `#[repr(transparent)]` code newtypes and returns a static-string error, dereferencing
//! nothing. That keeps every arm unit-testable without an encoder.

use gamut_core::Error;
use gamut_jxl_sys::encode::JxlEncoderError;

/// Translates the detailed [`JxlEncoderError`] behind a libjxl `JXL_ENC_ERROR` status into a
/// [`gamut_core::Error`] with a static `"JXL: …"` message.
///
/// The mapping mirrors libjxl's `JxlEncoderError` categories: out-of-memory and rejected input map
/// to [`Error::InvalidInput`], an unsupported configuration to [`Error::Unsupported`], and API misuse
/// (a bug in this crate — the wrapper is supposed to make every call in the right order) to
/// [`Error::InvalidInput`] after a `debug_assert!` that trips in debug builds. Any other code,
/// including the generic error, falls through to a generic [`Error::InvalidInput`].
pub(crate) fn map_encoder_error(err: JxlEncoderError) -> Error {
    match err {
        JxlEncoderError::OOM => Error::InvalidInput("JXL: encoder ran out of memory"),
        JxlEncoderError::BAD_INPUT => Error::InvalidInput("JXL: encoder rejected the input"),
        JxlEncoderError::NOT_SUPPORTED => {
            Error::Unsupported("JXL: encoder does not support this configuration")
        }
        JxlEncoderError::API_USAGE => {
            debug_assert!(false, "JXL: internal encoder API misuse (bug in gamut-jxl)");
            Error::InvalidInput("JXL: internal encoder API misuse (bug in gamut-jxl)")
        }
        // JBRD (JPEG-reconstruction) can't arise on the pixel-frame path, GENERIC is the catch-all,
        // and any future/unknown code is handled conservatively as a generic failure.
        _ => Error::InvalidInput("JXL: encoding failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oom_maps_to_invalid_input() {
        assert!(matches!(
            map_encoder_error(JxlEncoderError::OOM),
            Error::InvalidInput("JXL: encoder ran out of memory")
        ));
    }

    #[test]
    fn bad_input_maps_to_invalid_input() {
        assert!(matches!(
            map_encoder_error(JxlEncoderError::BAD_INPUT),
            Error::InvalidInput("JXL: encoder rejected the input")
        ));
    }

    #[test]
    fn not_supported_maps_to_unsupported() {
        assert!(matches!(
            map_encoder_error(JxlEncoderError::NOT_SUPPORTED),
            Error::Unsupported("JXL: encoder does not support this configuration")
        ));
    }

    #[test]
    fn generic_and_unknown_map_to_generic_failure() {
        assert!(matches!(
            map_encoder_error(JxlEncoderError::GENERIC),
            Error::InvalidInput("JXL: encoding failed")
        ));
        // JBRD and any unlisted code (ABI-representable via the transparent newtype) fall through.
        assert!(matches!(
            map_encoder_error(JxlEncoderError::JBRD),
            Error::InvalidInput("JXL: encoding failed")
        ));
        assert!(matches!(
            map_encoder_error(JxlEncoderError(9999)),
            Error::InvalidInput("JXL: encoding failed")
        ));
    }

    // API_USAGE is intentionally not exercised here: it fires a `debug_assert!` that would panic
    // under `cfg(debug_assertions)` (the default test profile). Its release-mode mapping is covered
    // by inspection; asserting it would require a release-profile test harness.
}
