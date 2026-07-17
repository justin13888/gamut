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

    /// Reads the stage-2 (linearized) image of the DNG at `path` — the SDK's application of the
    /// DNG Chapter-5 raw-to-linear mapping — into a freshly allocated `uint16` buffer
    /// (active-area `width * height * planes`, `0..=65535` encoding linear `0.0..=1.0`);
    /// `0` on success, else the SDK error code. Free with `gdng_free`.
    fn gdng_read_linear(
        path: *const c_char,
        out_w: *mut u32,
        out_h: *mut u32,
        out_planes: *mut u32,
        out_data: *mut *mut u16,
        out_len: *mut usize,
    ) -> c_int;

    /// Decodes a bare lossless-JPEG (SOF3) stream with the SDK's own codec into a freshly
    /// allocated interleaved `uint16` buffer of exactly `expected_samples`; `0` on success,
    /// else the SDK error code. Free with `gdng_free`.
    fn gdng_decode_lossless_jpeg(
        data: *const u8,
        len: usize,
        expected_samples: usize,
        out_data: *mut *mut u16,
        out_len: *mut usize,
    ) -> c_int;

    /// Releases a buffer returned by [`gdng_read_raw`] / [`gdng_read_linear`] /
    /// [`gdng_decode_lossless_jpeg`].
    fn gdng_free(data: *mut u16);
}

/// A 16-bit image as the Adobe DNG SDK reads it: interleaved samples and their geometry — the
/// stage-1 sensor values from [`read_raw_dng`], or the stage-2 linear encoding from
/// [`read_linear_dng`].
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

/// The directory of the SDK's own `sample_files/*.dng` conformance corpus, extracted from the
/// committed ZIP at build time — real Adobe-authored DNGs (JXL, ProfileGainTableMap, ImageStats,
/// ImageSequenceInfo, HDR/SDR profiles) for differential and byte-completeness testing.
#[must_use]
pub fn sample_files_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("OUT_DIR")).join("dng_sdk_extracted/dng_sdk_1_7_1/sample_files")
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

/// Runs one of the SDK's image-reading entry points over `bytes` written to a temporary file.
fn read_image(
    bytes: &[u8],
    // SAFETY contract: an `extern "C"` shim entry with the shared
    // `(path, w, h, planes, data, len)` signature that fills a `malloc`d `u16` buffer.
    entry: unsafe extern "C" fn(
        *const c_char,
        *mut u32,
        *mut u32,
        *mut u32,
        *mut *mut u16,
        *mut usize,
    ) -> c_int,
    what: &str,
) -> Result<AdobeRaw, String> {
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
        entry(
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
            "Adobe DNG SDK could not read the {what} image (code {code})"
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

/// Reads `bytes` as a DNG with the Adobe DNG SDK and returns its stage-1 raw samples.
///
/// # Errors
///
/// Returns an error message if the bytes cannot be written to a temporary file, or if the SDK
/// cannot read the raw image (with its numeric error code).
pub fn read_raw_dng(bytes: &[u8]) -> Result<AdobeRaw, String> {
    read_image(bytes, gdng_read_raw, "raw")
}

/// Reads `bytes` as a DNG with the Adobe DNG SDK and returns its **stage-2 (linearized)** image:
/// the SDK's application of the DNG Chapter-5 "Mapping Raw Values to Linear Reference Values"
/// pipeline (linearization table, black subtraction with deltas, rescale, clip).
///
/// The result is **active-area-sized** and encodes linear `0.0..=1.0` as `0..=65535`
/// (`linear = samples[i] as f64 / 65535.0`); the default SDK host preserves no black levels, so
/// `0` is black.
///
/// # Errors
///
/// Returns an error message if the bytes cannot be written to a temporary file, or if the SDK
/// cannot build/read the stage-2 image (with its numeric error code).
pub fn read_linear_dng(bytes: &[u8]) -> Result<AdobeRaw, String> {
    read_image(bytes, gdng_read_linear, "stage-2 linear")
}

/// Decodes a **bare lossless-JPEG (SOF3) stream** with the Adobe DNG SDK's own codec — the
/// reference for gamut-dng's T.81 process-14 decoder (predictors 1–7, point transform,
/// row-aligned restart intervals).
///
/// `expected_samples` is `width * height * components`; the SDK spools exactly that many
/// interleaved `u16` samples on success.
///
/// # Errors
///
/// Returns an error message (with the SDK's numeric error code) if the SDK rejects the stream
/// or decodes a different sample count.
pub fn decode_lossless_jpeg(stream: &[u8], expected_samples: usize) -> Result<Vec<u16>, String> {
    let mut data: *mut u16 = std::ptr::null_mut();
    let mut len: usize = 0;
    // SAFETY: `stream` outlives the call; on success `data`/`len` describe a `malloc`d buffer we
    // copy out of and then release with `gdng_free`.
    let code = unsafe {
        gdng_decode_lossless_jpeg(
            stream.as_ptr(),
            stream.len(),
            expected_samples,
            &mut data,
            &mut len,
        )
    };
    if code != 0 || data.is_null() {
        return Err(format!(
            "Adobe DNG SDK could not decode the lossless JPEG (code {code})"
        ));
    }
    // SAFETY: `data` points at `len` `u16`s the SDK just allocated; copy then free.
    let samples = unsafe { std::slice::from_raw_parts(data, len) }.to_vec();
    unsafe { gdng_free(data) };
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: the SDK links and runs, and rejects non-DNG bytes without crashing.
    #[test]
    fn rejects_non_dng_bytes() {
        assert!(validate_dng(b"this is not a DNG file").is_err());
        assert!(read_raw_dng(b"this is not a DNG file").is_err());
        assert!(decode_lossless_jpeg(b"not a JPEG", 4).is_err());
    }
}
