//! Dev-only DNG conformance oracle around a headless-built **Adobe DNG SDK 1.7.1**.
//!
//! gamut-dng's encoder must produce files the canonical reference implementation accepts.
//! [`validate_dng`] writes the bytes to a temporary file and runs the SDK's parse → build-negative
//! → read-stage-1 flow (the same one its `dng_validate` tool uses); it succeeds only if the SDK
//! reads the file without error. All `unsafe` FFI is confined to this crate.

use std::ffi::CString;
use std::os::raw::{c_char, c_int};

unsafe extern "C" {
    /// Returns `0` if the Adobe DNG SDK validates the DNG at `path`, else its error code.
    fn gdng_validate(path: *const c_char) -> c_int;

    /// Reads the stage-1 raw samples of the DNG at `path` into a freshly allocated `uint16` buffer
    /// (`width * height * planes`); `0` on success, else the SDK error code. Free with `gdng_free`.
    fn gdng_read_raw(
        path: *const c_char,
        out_w: *mut u32,
        out_h: *mut u32,
        out_planes: *mut u32,
        out_data: *mut *mut u16,
        out_len: *mut usize,
    ) -> c_int;

    /// Releases a buffer returned by [`gdng_read_raw`].
    fn gdng_free(data: *mut u16);
}

/// A raw image as the Adobe DNG SDK decodes it: the stage-1 sensor samples and their geometry.
#[derive(Debug, Clone)]
pub struct AdobeRaw {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Colour planes per pixel.
    pub planes: u32,
    /// Interleaved samples, row-major, `width * height * planes` long.
    pub samples: Vec<u16>,
}

/// Validates `bytes` as a DNG with the Adobe DNG SDK.
///
/// Returns `Ok(())` if the SDK parses the directories, builds a negative, and reads the raw image
/// without error; otherwise `Err` with the SDK error code.
///
/// # Errors
///
/// Returns an error message if the bytes cannot be written to a temporary file, or if the SDK
/// rejects the file (with its numeric error code).
pub fn validate_dng(bytes: &[u8]) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = dir.path().join("oracle.dng");
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    let cpath =
        CString::new(path.to_str().ok_or("non-UTF-8 temp path")?).map_err(|e| e.to_string())?;
    // SAFETY: `cpath` is a valid NUL-terminated path; the SDK only opens and reads the file at it.
    let code = unsafe { gdng_validate(cpath.as_ptr()) };
    if code == 0 {
        Ok(())
    } else {
        Err(format!(
            "Adobe DNG SDK rejected the file (error code {code})"
        ))
    }
}

/// Reads `bytes` as a DNG with the Adobe DNG SDK and returns its stage-1 raw samples.
///
/// # Errors
///
/// Returns an error message if the bytes cannot be written to a temporary file, or if the SDK
/// cannot read the raw image (with its numeric error code).
pub fn read_raw_dng(bytes: &[u8]) -> Result<AdobeRaw, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = dir.path().join("oracle.dng");
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    let cpath =
        CString::new(path.to_str().ok_or("non-UTF-8 temp path")?).map_err(|e| e.to_string())?;

    let (mut w, mut h, mut planes): (u32, u32, u32) = (0, 0, 0);
    let mut data: *mut u16 = std::ptr::null_mut();
    let mut len: usize = 0;
    // SAFETY: `cpath` is a valid NUL-terminated path; on success `data`/`len` describe a buffer the
    // SDK allocated with `malloc`, which we copy out of and then release with `gdng_free`.
    let code = unsafe {
        gdng_read_raw(
            cpath.as_ptr(),
            &mut w,
            &mut h,
            &mut planes,
            &mut data,
            &mut len,
        )
    };
    if code != 0 || data.is_null() {
        return Err(format!(
            "Adobe DNG SDK could not read the raw image (code {code})"
        ));
    }
    // SAFETY: `data` points at `len` `u16`s the SDK just allocated; copy then free.
    let samples = unsafe { std::slice::from_raw_parts(data, len) }.to_vec();
    unsafe { gdng_free(data) };
    Ok(AdobeRaw {
        width: w,
        height: h,
        planes,
        samples,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: the SDK links and runs, and rejects non-DNG bytes without crashing.
    #[test]
    fn rejects_non_dng_bytes() {
        assert!(validate_dng(b"this is not a DNG file").is_err());
        assert!(read_raw_dng(b"this is not a DNG file").is_err());
    }
}
