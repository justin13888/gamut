//! Dev-only differential oracle around a vendored, statically-linked **libtiff**.
//!
//! gamut's TIFF encoder must produce files that the canonical reference reader decodes back to the
//! same pixels, and its decoder must read files the reference writer produces. This crate wraps a
//! libtiff built from the `third_party/libtiff` submodule (all optional codecs disabled, so only
//! the built-in none/PackBits/LZW/CCITT schemes are available) behind a small, safe API:
//! [`decode_tiff`], [`encode_rgb8`], and [`encode_gray8`].
//!
//! libtiff's public API is file-based, so each call round-trips through a temporary file. All
//! `unsafe` FFI is confined to this crate.

#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case)]

use std::ffi::CString;
use std::os::raw::{c_int, c_void};
use std::path::Path;

mod sys {
    #![allow(dead_code)]
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

/// A TIFF compression scheme the oracle can write (a subset of libtiff's built-in schemes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    /// Uncompressed (`COMPRESSION_NONE`).
    None,
    /// PackBits run-length (`COMPRESSION_PACKBITS`).
    PackBits,
    /// LZW (`COMPRESSION_LZW`).
    Lzw,
}

impl Compression {
    fn code(self) -> u16 {
        match self {
            Compression::None => sys::COMPRESSION_NONE as u16,
            Compression::PackBits => sys::COMPRESSION_PACKBITS as u16,
            Compression::Lzw => sys::COMPRESSION_LZW as u16,
        }
    }
}

/// An image decoded by libtiff: interleaved 8-bit samples in raster order (no row padding).
pub struct DecodedImage {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Samples per pixel (1 for grayscale, 3 for RGB).
    pub samples_per_pixel: u16,
    /// `width * height * samples_per_pixel` interleaved 8-bit samples.
    pub pixels: Vec<u8>,
}

/// Decodes a TIFF byte stream with libtiff into interleaved 8-bit samples.
///
/// # Errors
///
/// Returns a message if the file cannot be written to a temp file, parsed, or is not 8-bit.
pub fn decode_tiff(bytes: &[u8]) -> Result<DecodedImage, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = dir.path().join("oracle.tiff");
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    let cpath = c_path(&path)?;
    // SAFETY: `cpath` is a valid NUL-terminated path; the TIFF handle is closed on every path.
    unsafe { decode_inner(&cpath) }
}

/// Encodes interleaved 8-bit RGB with libtiff at the given compression, returning the TIFF bytes.
///
/// # Errors
///
/// Returns a message if `pixels` does not match the dimensions or libtiff fails to write.
pub fn encode_rgb8(
    pixels: &[u8],
    width: u32,
    height: u32,
    compression: Compression,
) -> Result<Vec<u8>, String> {
    encode(
        pixels,
        width,
        height,
        3,
        sys::PHOTOMETRIC_RGB as u16,
        compression,
    )
}

/// Encodes 8-bit grayscale (`MINISBLACK`) with libtiff at the given compression.
///
/// # Errors
///
/// Returns a message if `pixels` does not match the dimensions or libtiff fails to write.
pub fn encode_gray8(
    pixels: &[u8],
    width: u32,
    height: u32,
    compression: Compression,
) -> Result<Vec<u8>, String> {
    encode(
        pixels,
        width,
        height,
        1,
        sys::PHOTOMETRIC_MINISBLACK as u16,
        compression,
    )
}

fn encode(
    pixels: &[u8],
    width: u32,
    height: u32,
    spp: u16,
    photometric: u16,
    compression: Compression,
) -> Result<Vec<u8>, String> {
    let row_bytes = (width as usize)
        .checked_mul(spp as usize)
        .ok_or("dimensions overflow")?;
    if pixels.len()
        != row_bytes
            .checked_mul(height as usize)
            .ok_or("dimensions overflow")?
    {
        return Err("pixel buffer does not match dimensions".into());
    }
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = dir.path().join("oracle.tiff");
    let cpath = c_path(&path)?;
    // SAFETY: `cpath` is valid; the handle is closed before we read the file back.
    unsafe {
        encode_inner(
            &cpath,
            pixels,
            width,
            height,
            spp,
            photometric,
            compression.code(),
        )?;
    }
    std::fs::read(&path).map_err(|e| e.to_string())
}

fn c_path(path: &Path) -> Result<CString, String> {
    CString::new(path.to_str().ok_or("non-UTF-8 temp path")?).map_err(|e| e.to_string())
}

unsafe fn decode_inner(cpath: &CString) -> Result<DecodedImage, String> {
    let mode = CString::new("r").map_err(|e| e.to_string())?;
    let t = unsafe { sys::TIFFOpen(cpath.as_ptr(), mode.as_ptr()) };
    if t.is_null() {
        return Err("TIFFOpen (read) failed".into());
    }
    let out = unsafe { read_scanlines(t) };
    unsafe { sys::TIFFClose(t) };
    out
}

unsafe fn read_scanlines(t: *mut sys::TIFF) -> Result<DecodedImage, String> {
    let mut width: u32 = 0;
    let mut height: u32 = 0;
    let mut spp: u16 = 1;
    let mut bps: u16 = 1;
    unsafe {
        if sys::TIFFGetField(t, sys::TIFFTAG_IMAGEWIDTH, &mut width as *mut u32) != 1 {
            return Err("missing ImageWidth".into());
        }
        if sys::TIFFGetField(t, sys::TIFFTAG_IMAGELENGTH, &mut height as *mut u32) != 1 {
            return Err("missing ImageLength".into());
        }
        sys::TIFFGetFieldDefaulted(t, sys::TIFFTAG_SAMPLESPERPIXEL, &mut spp as *mut u16);
        sys::TIFFGetFieldDefaulted(t, sys::TIFFTAG_BITSPERSAMPLE, &mut bps as *mut u16);
    }
    if bps != 8 {
        return Err(format!("expected 8-bit samples, got {bps}"));
    }
    let row_bytes = (width as usize) * (spp as usize);
    let scanline = unsafe { sys::TIFFScanlineSize(t) } as usize;
    let mut buf = vec![0u8; scanline.max(row_bytes).max(1)];
    let mut pixels = Vec::with_capacity(row_bytes * height as usize);
    for row in 0..height {
        let rc = unsafe { sys::TIFFReadScanline(t, buf.as_mut_ptr() as *mut c_void, row, 0) };
        if rc != 1 {
            return Err(format!("TIFFReadScanline failed at row {row}"));
        }
        pixels.extend_from_slice(&buf[..row_bytes]);
    }
    Ok(DecodedImage {
        width,
        height,
        samples_per_pixel: spp,
        pixels,
    })
}

unsafe fn encode_inner(
    cpath: &CString,
    pixels: &[u8],
    width: u32,
    height: u32,
    spp: u16,
    photometric: u16,
    compression: u16,
) -> Result<(), String> {
    let mode = CString::new("w").map_err(|e| e.to_string())?;
    let t = unsafe { sys::TIFFOpen(cpath.as_ptr(), mode.as_ptr()) };
    if t.is_null() {
        return Err("TIFFOpen (write) failed".into());
    }
    let result =
        unsafe { write_scanlines(t, pixels, width, height, spp, photometric, compression) };
    unsafe { sys::TIFFClose(t) };
    result
}

unsafe fn write_scanlines(
    t: *mut sys::TIFF,
    pixels: &[u8],
    width: u32,
    height: u32,
    spp: u16,
    photometric: u16,
    compression: u16,
) -> Result<(), String> {
    // uint32 fields take a `u32` vararg; uint16 fields are promoted to `c_int`.
    unsafe {
        sys::TIFFSetField(t, sys::TIFFTAG_IMAGEWIDTH, width);
        sys::TIFFSetField(t, sys::TIFFTAG_IMAGELENGTH, height);
        sys::TIFFSetField(t, sys::TIFFTAG_BITSPERSAMPLE, 8 as c_int);
        sys::TIFFSetField(t, sys::TIFFTAG_SAMPLESPERPIXEL, spp as c_int);
        sys::TIFFSetField(t, sys::TIFFTAG_PHOTOMETRIC, photometric as c_int);
        sys::TIFFSetField(t, sys::TIFFTAG_COMPRESSION, compression as c_int);
        sys::TIFFSetField(
            t,
            sys::TIFFTAG_PLANARCONFIG,
            sys::PLANARCONFIG_CONTIG as c_int,
        );
        let rps = sys::TIFFDefaultStripSize(t, 0);
        sys::TIFFSetField(t, sys::TIFFTAG_ROWSPERSTRIP, rps);
    }

    let row_bytes = (width as usize) * (spp as usize);
    let mut scratch = vec![0u8; row_bytes];
    for row in 0..height as usize {
        scratch.copy_from_slice(&pixels[row * row_bytes..(row + 1) * row_bytes]);
        let rc = unsafe {
            sys::TIFFWriteScanline(t, scratch.as_mut_ptr() as *mut c_void, row as u32, 0)
        };
        if rc != 1 {
            return Err(format!("TIFFWriteScanline failed at row {row}"));
        }
    }
    Ok(())
}
