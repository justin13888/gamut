//! Pure, safe sample/channel conversions between jxl-rs's decoded output and gamut's requested
//! pixel layout.
//!
//! jxl-rs writes the decoded frame as a flat byte buffer of interleaved samples in the stream's
//! *natural* colour layout (grayscale or RGB, optionally followed by one interleaved alpha
//! channel), in native byte order at the bit width the [`crate::decoder`] requested (8- or 16-bit).
//! This module turns those raw bytes into gamut's requested [`gamut_core::Pixel`] layout with three
//! primitive operations, in a single pass:
//!
//! - **byte → sample reassembly** (`u16` from a native-endian byte pair);
//! - **grayscale → RGB expansion** (replicate the luminance sample across R, G and B);
//! - **alpha reconciliation** — pad a missing alpha channel with an opaque value, or drop a present
//!   one — so a caller can request any colour-compatible layout regardless of what the stream
//!   carries.
//!
//! Grayscale is never *synthesised* from colour: the decoder rejects a colour-image-as-grayscale
//! request before ever calling in here, so `dst_color == 1` always implies `src_color == 1`.

/// A decoded-sample primitive: reassembled from native-endian bytes, with a known "fully opaque"
/// value for alpha padding. Sealed to `u8` and `u16` (the only sample widths gamut and jxl-rs
/// exchange), matching [`gamut_core::Sample`].
pub(crate) trait ConvSample: Copy {
    /// Bytes per sample in the raw jxl-rs output buffer.
    const BYTES: usize;
    /// The value representing fully-opaque alpha for this sample width (all bits set).
    const OPAQUE: Self;

    /// Reassembles one sample from the first [`ConvSample::BYTES`] bytes of `bytes`, interpreting a
    /// multi-byte sample in native byte order (jxl-rs is configured to emit native endianness).
    fn from_ne_bytes(bytes: &[u8]) -> Self;
}

impl ConvSample for u8 {
    const BYTES: usize = 1;
    const OPAQUE: Self = u8::MAX;

    fn from_ne_bytes(bytes: &[u8]) -> Self {
        bytes[0]
    }
}

impl ConvSample for u16 {
    const BYTES: usize = 2;
    const OPAQUE: Self = u16::MAX;

    fn from_ne_bytes(bytes: &[u8]) -> Self {
        u16::from_ne_bytes([bytes[0], bytes[1]])
    }
}

/// Converts `pixels` interleaved pixels from jxl-rs's raw byte output (`src`) into gamut's requested
/// sample layout (`dst`), in one pass.
///
/// - `src` holds `pixels * (src_color + src_alpha) * S::BYTES` bytes: `src_color` colour samples (1
///   for grayscale, 3 for RGB) followed by one alpha sample per pixel when `src_alpha`.
/// - `dst` receives `pixels * (dst_color + dst_alpha)` samples in the same interleaving.
///
/// Colour samples are copied straight across when `src_color == dst_color`, or replicated when
/// expanding grayscale (`src_color == 1`) to RGB (`dst_color == 3`). The alpha slot, when requested
/// (`dst_alpha`), takes the stream's alpha when present or [`ConvSample::OPAQUE`] otherwise; a
/// present alpha is simply not read when `dst_alpha` is `false` (dropped).
///
/// # Panics
///
/// Debug-asserts the layout invariants (`src`/`dst` lengths, and that grayscale is never synthesised
/// from colour). Lengths are guaranteed by the [`crate::decoder`] caller, so this never panics in
/// practice.
pub(crate) fn convert_into<S: ConvSample>(
    src: &[u8],
    src_color: usize,
    src_alpha: bool,
    dst: &mut [S],
    dst_color: usize,
    dst_alpha: bool,
    pixels: usize,
) {
    // Colour can be copied or expanded (1 -> 3), never reduced (3 -> 1): grayscale is not
    // synthesised from colour. The decoder enforces this before calling; assert it holds.
    debug_assert!(
        dst_color == src_color || (src_color == 1 && dst_color == 3),
        "unsupported colour conversion {src_color} -> {dst_color}",
    );
    let src_channels = src_color + usize::from(src_alpha);
    let dst_channels = dst_color + usize::from(dst_alpha);
    debug_assert_eq!(src.len(), pixels * src_channels * S::BYTES);
    debug_assert_eq!(dst.len(), pixels * dst_channels);

    for p in 0..pixels {
        let src_base = p * src_channels * S::BYTES;
        let dst_base = p * dst_channels;

        // Colour channels.
        if src_color == 1 {
            let g = S::from_ne_bytes(&src[src_base..]);
            // Grayscale straight through, or replicated across R/G/B.
            for c in 0..dst_color {
                dst[dst_base + c] = g;
            }
        } else {
            // src_color == 3 (RGB), which the invariant pairs only with dst_color == 3.
            for c in 0..dst_color {
                dst[dst_base + c] = S::from_ne_bytes(&src[src_base + c * S::BYTES..]);
            }
        }

        // Alpha channel: pad opaque when the request wants alpha the stream lacks; drop when the
        // request omits an alpha the stream has (by simply not reading it).
        if dst_alpha {
            let a = if src_alpha {
                S::from_ne_bytes(&src[src_base + src_color * S::BYTES..])
            } else {
                S::OPAQUE
            };
            dst[dst_base + dst_color] = a;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u8_from_ne_bytes_reads_first_byte() {
        assert_eq!(<u8 as ConvSample>::from_ne_bytes(&[0xAB, 0xCD]), 0xAB);
        assert_eq!(u8::OPAQUE, 0xFF);
        assert_eq!(u8::BYTES, 1);
    }

    #[test]
    fn u16_reassembles_native_endian_pair() {
        // Build the native-endian encoding of 0x1234 and confirm reassembly is exact.
        let bytes = 0x1234u16.to_ne_bytes();
        assert_eq!(<u16 as ConvSample>::from_ne_bytes(&bytes), 0x1234);
        // Extra trailing bytes are ignored (only the first two are read).
        let mut padded = bytes.to_vec();
        padded.push(0xFF);
        assert_eq!(<u16 as ConvSample>::from_ne_bytes(&padded), 0x1234);
        assert_eq!(u16::OPAQUE, 0xFFFF);
        assert_eq!(u16::BYTES, 2);
    }

    /// Runs one 2-pixel `u8` conversion case and returns the produced destination samples.
    fn run_u8(
        src: &[u8],
        src_color: usize,
        src_alpha: bool,
        dst_color: usize,
        dst_alpha: bool,
    ) -> Vec<u8> {
        let pixels = 2;
        let mut dst = vec![0u8; pixels * (dst_color + usize::from(dst_alpha))];
        convert_into::<u8>(
            src, src_color, src_alpha, &mut dst, dst_color, dst_alpha, pixels,
        );
        dst
    }

    #[test]
    fn gray_identity() {
        // G -> G: straight copy.
        assert_eq!(run_u8(&[10, 20], 1, false, 1, false), vec![10, 20]);
    }

    #[test]
    fn gray_to_gray_alpha_pads_opaque() {
        // G -> GA: opaque alpha appended.
        assert_eq!(
            run_u8(&[10, 20], 1, false, 1, true),
            vec![10, 0xFF, 20, 0xFF]
        );
    }

    #[test]
    fn gray_alpha_to_gray_drops_alpha() {
        // GA -> G: alpha read past, not written.
        assert_eq!(run_u8(&[10, 5, 20, 6], 1, true, 1, false), vec![10, 20]);
    }

    #[test]
    fn gray_alpha_identity_preserves_alpha() {
        assert_eq!(
            run_u8(&[10, 5, 20, 6], 1, true, 1, true),
            vec![10, 5, 20, 6]
        );
    }

    #[test]
    fn gray_expands_to_rgb() {
        // G -> RGB: luminance replicated across three channels.
        assert_eq!(
            run_u8(&[10, 20], 1, false, 3, false),
            vec![10, 10, 10, 20, 20, 20]
        );
    }

    #[test]
    fn gray_expands_to_rgba_with_opaque_alpha() {
        assert_eq!(
            run_u8(&[10, 20], 1, false, 3, true),
            vec![10, 10, 10, 0xFF, 20, 20, 20, 0xFF]
        );
    }

    #[test]
    fn gray_alpha_expands_to_rgb_dropping_alpha() {
        assert_eq!(
            run_u8(&[10, 5, 20, 6], 1, true, 3, false),
            vec![10, 10, 10, 20, 20, 20]
        );
    }

    #[test]
    fn gray_alpha_expands_to_rgba_keeping_alpha() {
        assert_eq!(
            run_u8(&[10, 5, 20, 6], 1, true, 3, true),
            vec![10, 10, 10, 5, 20, 20, 20, 6]
        );
    }

    #[test]
    fn rgb_identity() {
        assert_eq!(
            run_u8(&[1, 2, 3, 4, 5, 6], 3, false, 3, false),
            vec![1, 2, 3, 4, 5, 6]
        );
    }

    #[test]
    fn rgb_to_rgba_pads_opaque() {
        assert_eq!(
            run_u8(&[1, 2, 3, 4, 5, 6], 3, false, 3, true),
            vec![1, 2, 3, 0xFF, 4, 5, 6, 0xFF]
        );
    }

    #[test]
    fn rgba_to_rgb_drops_alpha() {
        assert_eq!(
            run_u8(&[1, 2, 3, 9, 4, 5, 6, 8], 3, true, 3, false),
            vec![1, 2, 3, 4, 5, 6]
        );
    }

    #[test]
    fn rgba_identity() {
        assert_eq!(
            run_u8(&[1, 2, 3, 9, 4, 5, 6, 8], 3, true, 3, true),
            vec![1, 2, 3, 9, 4, 5, 6, 8]
        );
    }

    #[test]
    fn u16_rgba_identity_roundtrips_through_native_bytes() {
        // Two RGBA16 pixels, laid out as native-endian bytes exactly as jxl-rs would emit them.
        let samples: [u16; 8] = [
            0x1111, 0x2222, 0x3333, 0xFFFF, 0x4444, 0x5555, 0x6666, 0x8000,
        ];
        let mut src = Vec::new();
        for s in samples {
            src.extend_from_slice(&s.to_ne_bytes());
        }
        let mut dst = vec![0u16; 8];
        convert_into::<u16>(&src, 3, true, &mut dst, 3, true, 2);
        assert_eq!(dst, samples);
    }

    #[test]
    fn u16_gray_expands_to_rgba_with_full_opaque() {
        let samples: [u16; 2] = [0x0102, 0x0304];
        let mut src = Vec::new();
        for s in samples {
            src.extend_from_slice(&s.to_ne_bytes());
        }
        let mut dst = vec![0u16; 8];
        convert_into::<u16>(&src, 1, false, &mut dst, 3, true, 2);
        assert_eq!(
            dst,
            vec![
                0x0102, 0x0102, 0x0102, 0xFFFF, 0x0304, 0x0304, 0x0304, 0xFFFF
            ]
        );
    }
}
