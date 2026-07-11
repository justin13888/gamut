//! Test-only oracle: a safe wrapper over the reference libjxl **decoder** exposed by
//! `gamut-jxl-sys`, used to read back what `gamut-jxl`'s encoder produced.
//!
//! Integration-test code is a separate crate and is not bound by `gamut-jxl`'s
//! `#![deny(unsafe_code)]`, so the raw FFI calls live here directly; `#![allow(dead_code)]` keeps the
//! shared helper usable from test files that only need part of it.
#![allow(unsafe_code)]
#![allow(dead_code)]

use core::ffi::c_void;
use core::mem::MaybeUninit;

use gamut_jxl_sys::{decode as dec, types as ty};

/// Decoded sample storage, matching the encoded bit depth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedSamples {
    /// 8-bit interleaved samples.
    U8(Vec<u8>),
    /// 16-bit interleaved samples (native-endian, as libjxl wrote them).
    U16(Vec<u16>),
}

/// A libjxl-decoded image: its dimensions, interleaved channel count, and samples.
#[derive(Debug, Clone)]
pub struct Decoded {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Interleaved channels per pixel (colour + alpha).
    pub num_channels: u32,
    /// The decoded samples.
    pub samples: DecodedSamples,
}

/// RAII owner of a libjxl decoder handle.
struct Decoder(*mut dec::JxlDecoder);

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe { dec::JxlDecoderDestroy(self.0) };
    }
}

/// Decodes any JPEG XL stream (bare codestream or ISO BMFF container) with the reference libjxl
/// decoder, returning the image in the natural layout of the codestream: `num_color_channels` plus a
/// single alpha channel if any extra channel is present, at `UINT16` if the coded bit depth exceeds 8
/// and `UINT8` otherwise, in native endianness.
///
/// # Panics
///
/// Panics (with a descriptive message) on any decoder error — appropriate for a test oracle.
pub fn decode(data: &[u8]) -> Decoded {
    let handle = unsafe { dec::JxlDecoderCreate(core::ptr::null()) };
    assert!(!handle.is_null(), "JxlDecoderCreate returned null");
    let decoder = Decoder(handle);

    let events = dec::JxlDecoderStatus::BASIC_INFO.0 | dec::JxlDecoderStatus::FULL_IMAGE.0;
    let st = unsafe { dec::JxlDecoderSubscribeEvents(decoder.0, events) };
    assert_eq!(st, dec::JxlDecoderStatus::SUCCESS, "SubscribeEvents failed");
    let st = unsafe { dec::JxlDecoderSetInput(decoder.0, data.as_ptr(), data.len()) };
    assert_eq!(st, dec::JxlDecoderStatus::SUCCESS, "SetInput failed");
    unsafe { dec::JxlDecoderCloseInput(decoder.0) };

    let mut width = 0u32;
    let mut height = 0u32;
    let mut num_channels = 0u32;
    let mut use_u16 = false;
    let mut format: Option<ty::JxlPixelFormat> = None;
    let mut out_u8: Vec<u8> = Vec::new();
    let mut out_u16: Vec<u16> = Vec::new();

    loop {
        let status = unsafe { dec::JxlDecoderProcessInput(decoder.0) };
        if status == dec::JxlDecoderStatus::SUCCESS {
            break;
        } else if status == dec::JxlDecoderStatus::BASIC_INFO {
            let mut bi = MaybeUninit::<ty::JxlBasicInfo>::uninit();
            let st = unsafe { dec::JxlDecoderGetBasicInfo(decoder.0, bi.as_mut_ptr()) };
            assert_eq!(st, dec::JxlDecoderStatus::SUCCESS, "GetBasicInfo failed");
            let bi = unsafe { bi.assume_init() };
            width = bi.xsize;
            height = bi.ysize;
            let alpha = u32::from(bi.num_extra_channels > 0);
            num_channels = bi.num_color_channels + alpha;
            use_u16 = bi.bits_per_sample > 8;
            let data_type = if use_u16 {
                ty::JxlDataType::UINT16
            } else {
                ty::JxlDataType::UINT8
            };
            format = Some(ty::JxlPixelFormat {
                num_channels,
                data_type,
                endianness: ty::JxlEndianness::NATIVE,
                align: 0,
            });
        } else if status == dec::JxlDecoderStatus::NEED_IMAGE_OUT_BUFFER {
            let fmt = format.expect("BASIC_INFO must precede NEED_IMAGE_OUT_BUFFER");
            let mut size = 0usize;
            let st = unsafe { dec::JxlDecoderImageOutBufferSize(decoder.0, &fmt, &mut size) };
            assert_eq!(
                st,
                dec::JxlDecoderStatus::SUCCESS,
                "ImageOutBufferSize failed"
            );
            let st = if use_u16 {
                out_u16 = vec![0u16; size / 2];
                unsafe {
                    dec::JxlDecoderSetImageOutBuffer(
                        decoder.0,
                        &fmt,
                        out_u16.as_mut_ptr().cast::<c_void>(),
                        size,
                    )
                }
            } else {
                out_u8 = vec![0u8; size];
                unsafe {
                    dec::JxlDecoderSetImageOutBuffer(
                        decoder.0,
                        &fmt,
                        out_u8.as_mut_ptr().cast::<c_void>(),
                        size,
                    )
                }
            };
            assert_eq!(
                st,
                dec::JxlDecoderStatus::SUCCESS,
                "SetImageOutBuffer failed"
            );
        } else if status == dec::JxlDecoderStatus::FULL_IMAGE {
            // The frame is fully written into our buffer; keep processing to reach SUCCESS.
        } else {
            panic!("unexpected decoder status: {status:?}");
        }
    }

    let samples = if use_u16 {
        DecodedSamples::U16(out_u16)
    } else {
        DecodedSamples::U8(out_u8)
    };
    Decoded {
        width,
        height,
        num_channels,
        samples,
    }
}

/// Peak signal-to-noise ratio in dB over two equal-length `u8` sample sets, treating them as a flat
/// signal in `0..=255`. Returns `f64::INFINITY` when identical.
pub fn psnr_u8(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len(), "PSNR inputs differ in length");
    let mut sse = 0.0f64;
    for (&x, &y) in a.iter().zip(b) {
        let d = f64::from(x) - f64::from(y);
        sse += d * d;
    }
    if sse == 0.0 {
        return f64::INFINITY;
    }
    let mse = sse / a.len() as f64;
    20.0 * 255.0f64.log10() - 10.0 * mse.log10()
}
