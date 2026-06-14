//! Dev-only FFI oracle over a vendored, statically-linked exiv2 — the reference for the legacy
//! IPTC-IIM binary carrier.
//!
//! Used only by gamut-iptc's differential tests; never by shipped code. exiv2 is built with XMP
//! disabled (see the build script), so this oracle covers the IIM dataset stream and the Photoshop
//! IRB, not IPTC-in-XMP. `unsafe` is confined here, across the FFI boundary to the C++ shim.

use std::os::raw::c_int;

unsafe extern "C" {
    fn gex_iim_count(data: *const u8, len: usize) -> i64;
    fn gex_iim_dataset(
        data: *const u8,
        len: usize,
        index: usize,
        record: *mut u16,
        tag: *mut u16,
        out: *mut *mut u8,
        out_len: *mut usize,
    ) -> c_int;
    fn gex_iim_reencode(
        data: *const u8,
        len: usize,
        out: *mut *mut u8,
        out_len: *mut usize,
    ) -> c_int;
    fn gex_irb_iptc(data: *const u8, len: usize, out: *mut *mut u8, out_len: *mut usize) -> c_int;
    fn gex_free(p: *mut u8);
}

/// One IIM dataset as exiv2 decodes it: its record number, dataset (tag) number, and raw value
/// octets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleDataset {
    /// The IIM record number.
    pub record: u16,
    /// The dataset (tag) number within the record.
    pub tag: u16,
    /// The raw value octets, as exiv2 serializes them (big-endian for numeric values).
    pub value: Vec<u8>,
}

/// Copies an allocation returned by the shim into a `Vec` and frees it.
///
/// # Safety
///
/// `ptr`/`len` must be a buffer the shim allocated via `malloc` and handed back.
unsafe fn take(ptr: *mut u8, len: usize) -> Vec<u8> {
    let v = unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec();
    unsafe { gex_free(ptr) };
    v
}

/// Decodes an IIM dataset stream with exiv2, or `None` if exiv2 rejects it.
#[must_use]
pub fn parse_iim(data: &[u8]) -> Option<Vec<OracleDataset>> {
    let count = unsafe { gex_iim_count(data.as_ptr(), data.len()) };
    if count < 0 {
        return None;
    }
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        let mut record = 0u16;
        let mut tag = 0u16;
        let mut buf: *mut u8 = std::ptr::null_mut();
        let mut len = 0usize;
        let rc = unsafe {
            gex_iim_dataset(
                data.as_ptr(),
                data.len(),
                i,
                &mut record,
                &mut tag,
                &mut buf,
                &mut len,
            )
        };
        if rc != 0 || buf.is_null() {
            return None;
        }
        let value = unsafe { take(buf, len) };
        out.push(OracleDataset { record, tag, value });
    }
    Some(out)
}

/// Decodes then re-encodes an IIM dataset stream with exiv2 (the reference round-trip), or `None` on
/// error.
#[must_use]
pub fn reencode_iim(data: &[u8]) -> Option<Vec<u8>> {
    let mut buf: *mut u8 = std::ptr::null_mut();
    let mut len = 0usize;
    let rc = unsafe { gex_iim_reencode(data.as_ptr(), data.len(), &mut buf, &mut len) };
    if rc != 0 || buf.is_null() {
        return None;
    }
    Some(unsafe { take(buf, len) })
}

/// Locates the IPTC (`0x0404`) IIM payload within a Photoshop `8BIM` stream, or `None` if absent or
/// invalid.
#[must_use]
pub fn locate_iptc_irb(data: &[u8]) -> Option<Vec<u8>> {
    let mut buf: *mut u8 = std::ptr::null_mut();
    let mut len = 0usize;
    let rc = unsafe { gex_irb_iptc(data.as_ptr(), data.len(), &mut buf, &mut len) };
    if rc != 0 || buf.is_null() {
        return None;
    }
    Some(unsafe { take(buf, len) })
}
