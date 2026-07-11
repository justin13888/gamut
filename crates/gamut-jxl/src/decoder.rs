//! The [`JxlDecoder`]: a safe front end over the pure-Rust jxl-rs decoder (the [`jxl`] crate).
//!
//! Decoding drives jxl-rs's typestate API (`Initialized → WithImageInfo → WithFrameInfo`) once per
//! call, requesting the stream's *natural* colour layout at the caller's requested bit width, then
//! reconciling channels to the requested [`gamut_core::Pixel`] layout in [`crate::convert`]. All of
//! that is 100% safe Rust: this module contains no `unsafe`.

use gamut_core::{
    DecodeImage, Dimensions, Error, Gray8, Gray16, GrayAlpha8, GrayAlpha16, ImageBuf, Pixel,
    Result, Rgb8, Rgb16, Rgba8, Rgba16,
};
use jxl::api::states::Initialized;
use jxl::api::{
    Endianness, JxlColorType, JxlDataFormat, JxlDecoder as JxlRsDecoder, JxlDecoderOptions,
    JxlOutputBuffer, JxlPixelFormat, ProcessingResult,
};
use jxl::headers::extra_channels::ExtraChannel;

use crate::convert::{ConvSample, convert_into};
use crate::error::map_decode_error;

/// Upper bound on decoded size, in **pixels × channels**, enforced by jxl-rs's
/// [`JxlDecoderOptions::pixel_limit`]. `1 << 28` (≈268M) samples caps a hostile stream's memory use
/// (a 16-bit RGBA image at this limit is ~512 MiB of output) while comfortably admitting every
/// realistic still image. The limit is deliberately generous and additive to lower later.
const PIXEL_LIMIT: usize = 1 << 28;

/// A pure-Rust JPEG XL decoder.
///
/// Decodes both JPEG XL framings — a bare codestream and the ISO BMFF `.jxl` container — into any of
/// the eight supported pixel layouts (8/16-bit grayscale, gray+alpha, RGB, RGBA) through the
/// [`DecodeImage`](gamut_core::DecodeImage) trait. Where the requested layout and the stream differ,
/// the decoder converts internally: grayscale expands to RGB, a missing alpha channel is padded
/// opaque, and a present-but-unwanted alpha is dropped. It deliberately refuses to *guess*: a colour
/// image cannot be decoded as grayscale, animated input is rejected, and premultiplied (associated)
/// alpha is rejected — each an [`Error::Unsupported`] that a later version may relax.
///
/// The type is a zero-sized handle; construct it with [`JxlDecoder::new`] or [`Default`]. It holds no
/// configuration today — the private field reserves room to add decode options (e.g. a caller-tunable
/// pixel limit) without a breaking change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JxlDecoder {
    _private: (),
}

impl JxlDecoder {
    /// Creates a decoder with the default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self { _private: () }
    }
}

/// The raw output of one decoded frame: jxl-rs's interleaved byte buffer in the stream's natural
/// colour layout, plus the metadata [`convert_into`] needs to reshape it.
struct RawFrame {
    /// The decoded image dimensions (display orientation).
    dims: Dimensions,
    /// The interleaved sample bytes, native byte order, occupying `bytes[offset..offset + len]`.
    bytes: Vec<u8>,
    /// Start of the used region within `bytes` (0 or 1; a parity offset that guarantees 2-byte row
    /// alignment for 16-bit output — see [`aligned_backing`]).
    offset: usize,
    /// Length of the used region within `bytes`.
    len: usize,
    /// Colour samples per pixel in `bytes`: 1 (grayscale) or 3 (RGB).
    src_color: usize,
    /// Whether an interleaved alpha sample follows the colour samples in `bytes`.
    src_alpha: bool,
}

impl RawFrame {
    /// The used byte region.
    fn samples(&self) -> &[u8] {
        &self.bytes[self.offset..self.offset + self.len]
    }
}

/// Whether a decoded output buffer holding `bytes_per_sample`-wide samples needs a 2-byte-aligned
/// base address.
///
/// jxl-rs's interleaved 16-bit output fast path reinterprets the byte buffer as `&mut [u16]` only
/// when it is 2-byte aligned, and **silently writes nothing** into an unaligned buffer (it returns a
/// zero written-length), so a decoded 16-bit image would come back as all-zero garbage. 8-bit output
/// is written byte-wise and has no alignment requirement. Hence exactly the 2-byte sample width needs
/// alignment. Factored out so the decision is unit-tested rather than buried in a call argument.
fn needs_row_alignment(bytes_per_sample: usize) -> bool {
    bytes_per_sample == 2
}

/// Allocates a zeroed byte buffer of `total` bytes and returns it with an offset into it such that
/// `buf[offset..]` begins at an even address when `align2` is set.
///
/// jxl-rs requires each output row of a 16-bit (2-byte) pixel format to start on a 2-byte boundary,
/// or it may panic. Our rows are tightly packed at an even stride (`width × channels × 2`), so it
/// suffices to make the buffer's *base* even: we over-allocate by one byte and skip the first byte
/// when the allocation happens to land on an odd address. Reading the pointer's address is a plain
/// `as` cast — no `unsafe`, no dereference — so the crate stays `#![deny(unsafe_code)]`.
fn aligned_backing(total: usize, align2: bool) -> (Vec<u8>, usize) {
    if !align2 {
        return (vec![0u8; total], 0);
    }
    let buf = vec![0u8; total + 1];
    // `base & 1` is 1 for an odd base (skip one byte to reach the next even address) and 0 for an
    // even base (already aligned).
    let offset = (buf.as_ptr() as usize) & 1;
    (buf, offset)
}

/// Decodes `data` into jxl-rs's natural interleaved layout at `S`'s bit width, applying the decode
/// policies (pixel limit, animation/premultiplied/color-as-gray rejection, truncation) before any
/// channel reshaping.
///
/// `dst_is_gray_family` is the requested layout's colour family; a grayscale request against a colour
/// stream is rejected here (refusing to guess a luminance). `data_format` selects the output bit
/// width (8- or 16-bit, native endianness) and must agree with `S`.
fn decode_raw<S: ConvSample>(
    data: &[u8],
    dst_is_gray_family: bool,
    data_format: JxlDataFormat,
) -> Result<RawFrame> {
    // `JxlDecoderOptions` is `#[non_exhaustive]`, so it is built from its `Default` and then the
    // one public field we care about is set.
    let mut options = JxlDecoderOptions::default();
    options.pixel_limit = Some(PIXEL_LIMIT);

    let mut input: &[u8] = data;

    // Initialized -> WithImageInfo: parse the file/frame headers.
    let decoder = JxlRsDecoder::<Initialized>::new(options);
    let mut decoder = match decoder.process(&mut input).map_err(map_decode_error)? {
        ProcessingResult::Complete { result } => result,
        ProcessingResult::NeedsMoreInput { .. } => return Err(truncated()),
    };

    // Read everything we need from the image info, then drop the borrows before reconfiguring.
    let (size, num_extra, stream_is_gray, stream_has_alpha, premultiplied) = {
        let basic = decoder.basic_info();
        if basic.animation.is_some() {
            return Err(Error::Unsupported("JXL: animated JPEG XL is not supported"));
        }
        let alpha = basic
            .extra_channels
            .iter()
            .find(|c| c.ec_type == ExtraChannel::Alpha);
        let premultiplied = alpha.is_some_and(|c| c.alpha_associated);
        // The default pixel format's colour type reflects the stream's colour space (grayscale vs
        // RGB) and never carries alpha, so it is the cleanest signal of "is this a colour image?".
        let stream_is_gray = decoder.current_pixel_format().color_type.is_grayscale();
        (
            basic.size,
            basic.extra_channels.len(),
            stream_is_gray,
            alpha.is_some(),
            premultiplied,
        )
    };

    if premultiplied {
        return Err(Error::Unsupported(
            "JXL: premultiplied (associated) alpha is not supported",
        ));
    }
    if dst_is_gray_family && !stream_is_gray {
        return Err(Error::Unsupported(
            "JXL: cannot decode a color image as grayscale",
        ));
    }

    // jxl-rs reports usize dimensions; gamut carries u32. A stream claiming a dimension beyond u32
    // is malformed for gamut's buffers (and far past the pixel limit anyway).
    let (Ok(width), Ok(height)) = (u32::try_from(size.0), u32::try_from(size.1)) else {
        return Err(Error::InvalidInput("JXL: image dimensions overflow"));
    };
    let dims = Dimensions::new(width, height)?;

    // Request the stream's natural colour layout; `convert_into` reshapes it to the caller's layout.
    let src_color = if stream_is_gray { 1 } else { 3 };
    let src_alpha = stream_has_alpha;
    let color_type = match (stream_is_gray, stream_has_alpha) {
        (true, false) => JxlColorType::Grayscale,
        (true, true) => JxlColorType::GrayscaleAlpha,
        (false, false) => JxlColorType::Rgb,
        (false, true) => JxlColorType::Rgba,
    };
    decoder.set_pixel_format(JxlPixelFormat {
        color_type,
        color_data_format: Some(data_format),
        // Every extra channel is ignored (`None`): alpha, when wanted, arrives interleaved via the
        // colour type above. The vector length must still match the stream's extra-channel count.
        extra_channel_format: vec![None; num_extra],
    });

    // Size the single interleaved output buffer, guarding the byte arithmetic against overflow.
    let src_channels = src_color + usize::from(src_alpha);
    let bytes_per_row = (width as usize)
        .checked_mul(src_channels)
        .and_then(|n| n.checked_mul(S::BYTES))
        .ok_or(Error::InvalidInput("JXL: image dimensions overflow"))?;
    let total = bytes_per_row
        .checked_mul(height as usize)
        .ok_or(Error::InvalidInput("JXL: image dimensions overflow"))?;
    let (mut backing, offset) = aligned_backing(total, needs_row_alignment(S::BYTES));

    // WithImageInfo -> WithFrameInfo: parse the frame header.
    let decoder = match decoder.process(&mut input).map_err(map_decode_error)? {
        ProcessingResult::Complete { result } => result,
        ProcessingResult::NeedsMoreInput { .. } => return Err(truncated()),
    };

    // WithFrameInfo -> WithImageInfo: render pixels into our buffer.
    {
        let buf = JxlOutputBuffer::new(
            &mut backing[offset..offset + total],
            height as usize,
            bytes_per_row,
        );
        let mut buffers = [buf];
        match decoder
            .process(&mut input, &mut buffers)
            .map_err(map_decode_error)?
        {
            ProcessingResult::Complete { .. } => {}
            ProcessingResult::NeedsMoreInput { .. } => return Err(truncated()),
        }
    }

    Ok(RawFrame {
        dims,
        bytes: backing,
        offset,
        len: total,
        src_color,
        src_alpha,
    })
}

/// The error for a codestream that ends before a full image could be decoded. jxl-rs signals this as
/// `NeedsMoreInput`; since the decoder is fed the whole buffer at once, needing more means the input
/// was truncated.
fn truncated() -> Error {
    Error::InvalidInput("JXL: truncated codestream")
}

/// Decodes `data` into a fresh [`ImageBuf`] of layout `P`.
fn decode_to_buf<P: Pixel>(
    data: &[u8],
    dst_is_gray_family: bool,
    data_format: JxlDataFormat,
    dst_color: usize,
    dst_alpha: bool,
) -> Result<ImageBuf<P>>
where
    P::Sample: ConvSample,
{
    let raw = decode_raw::<P::Sample>(data, dst_is_gray_family, data_format)?;
    let pixels = raw
        .dims
        .num_pixels()
        .ok_or(Error::InvalidInput("JXL: image dimensions overflow"))?;
    let mut out = vec![P::Sample::default(); pixels * (dst_color + usize::from(dst_alpha))];
    convert_into::<P::Sample>(
        raw.samples(),
        raw.src_color,
        raw.src_alpha,
        &mut out,
        dst_color,
        dst_alpha,
        pixels,
    );
    ImageBuf::<P>::new(out, raw.dims)
}

/// Decodes `data` into `dst`, reusing its sample allocation when the decoded dimensions match.
fn decode_into_buf<P: Pixel>(
    data: &[u8],
    dst_is_gray_family: bool,
    data_format: JxlDataFormat,
    dst_color: usize,
    dst_alpha: bool,
    dst: &mut ImageBuf<P>,
) -> Result<()>
where
    P::Sample: ConvSample,
{
    let raw = decode_raw::<P::Sample>(data, dst_is_gray_family, data_format)?;
    let pixels = raw
        .dims
        .num_pixels()
        .ok_or(Error::InvalidInput("JXL: image dimensions overflow"))?;
    if dst.dimensions() == raw.dims {
        // Same geometry: convert straight into the existing storage (its length is invariant).
        convert_into::<P::Sample>(
            raw.samples(),
            raw.src_color,
            raw.src_alpha,
            dst.as_mut_samples(),
            dst_color,
            dst_alpha,
            pixels,
        );
    } else {
        let mut out = vec![P::Sample::default(); pixels * (dst_color + usize::from(dst_alpha))];
        convert_into::<P::Sample>(
            raw.samples(),
            raw.src_color,
            raw.src_alpha,
            &mut out,
            dst_color,
            dst_alpha,
            pixels,
        );
        *dst = ImageBuf::<P>::new(out, raw.dims)?;
    }
    Ok(())
}

/// The native-endian 8-bit output format.
fn u8_format() -> JxlDataFormat {
    JxlDataFormat::U8 { bit_depth: 8 }
}

/// The native-endian 16-bit output format.
fn u16_format() -> JxlDataFormat {
    JxlDataFormat::U16 {
        endianness: Endianness::native(),
        bit_depth: 16,
    }
}

/// Implements [`DecodeImage`] for each supported layout, wiring its colour family, channel counts and
/// output bit width into the shared generic path.
macro_rules! impl_decode_image {
    ($($pixel:ty => $format:expr, $gray_family:expr, $dst_color:expr, $dst_alpha:expr;)*) => {$(
        impl DecodeImage<$pixel> for JxlDecoder {
            fn decode_image(&self, data: &[u8]) -> Result<ImageBuf<$pixel>> {
                decode_to_buf::<$pixel>(data, $gray_family, $format, $dst_color, $dst_alpha)
            }

            fn decode_image_into(&self, data: &[u8], dst: &mut ImageBuf<$pixel>) -> Result<()> {
                decode_into_buf::<$pixel>(data, $gray_family, $format, $dst_color, $dst_alpha, dst)
            }
        }
    )*};
}

impl_decode_image! {
    Gray8       => u8_format(),  true,  1, false;
    GrayAlpha8  => u8_format(),  true,  1, true;
    Rgb8        => u8_format(),  false, 3, false;
    Rgba8       => u8_format(),  false, 3, true;
    Gray16      => u16_format(), true,  1, false;
    GrayAlpha16 => u16_format(), true,  1, true;
    Rgb16       => u16_format(), false, 3, false;
    Rgba16      => u16_format(), false, 3, true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_and_default_are_equal() {
        assert_eq!(JxlDecoder::new(), JxlDecoder::default());
    }

    #[test]
    fn needs_row_alignment_only_for_two_byte_samples() {
        // 16-bit (2-byte) output must be aligned; 8-bit (1-byte) must not force alignment. This pins
        // the decision that guards jxl-rs's alignment-sensitive interleaved-u16 fast path.
        assert!(!needs_row_alignment(1), "8-bit needs no alignment");
        assert!(needs_row_alignment(2), "16-bit needs 2-byte alignment");
    }

    #[test]
    fn aligned_backing_u8_is_tight_and_zero_offset() {
        let (buf, off) = aligned_backing(10, false);
        assert_eq!(buf.len(), 10);
        assert_eq!(off, 0);
    }

    #[test]
    fn aligned_backing_u16_starts_even() {
        let (buf, off) = aligned_backing(10, true);
        // One spare byte for the parity shift, and the used region begins on an even address.
        assert_eq!(buf.len(), 11);
        assert!(off <= 1);
        assert_eq!((buf.as_ptr() as usize + off) % 2, 0);
        assert!(off + 10 <= buf.len());
    }

    #[test]
    fn truncated_is_invalid_input() {
        assert!(matches!(truncated(), Error::InvalidInput(_)));
    }

    #[test]
    fn empty_and_garbage_input_error_without_panicking() {
        // Enough of the decode entry path to prove it returns typed errors on junk rather than
        // panicking; full corpus coverage lives in the robustness test unit.
        let dec = JxlDecoder::new();
        assert!(<JxlDecoder as DecodeImage<Rgba8>>::decode_image(&dec, &[]).is_err());
        assert!(
            <JxlDecoder as DecodeImage<Rgb8>>::decode_image(&dec, &[0x00, 0x01, 0x02, 0x03])
                .is_err()
        );
    }
}
