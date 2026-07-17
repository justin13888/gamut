//! Dev-only differential oracle around a vendored, statically-linked **libavif**.
//!
//! gamut's AVIF encoder must produce containers that a real AVIF reader decodes back to the same
//! pixels the AV1 layer reconstructed. This crate parses + decodes a full `.avif` byte stream with
//! libavif (dav1d backend) and hands back the decoded YUV planes, so the end-to-end roundtrip test
//! can compare them to the source (lossless) or the encoder's reconstruction (lossy) — without
//! depending on an `avifdec` binary being installed. The decoded planes are the exact bytes the AV1
//! decoder produced (no RGB color conversion), mirroring the old `avifdec` Y4M path. The C libraries
//! are built from the `third_party/libavif` and `third_party/dav1d` git submodules by `build.rs`.
//!
//! For the container **decode** surface (issue #250) two further entry points serve the
//! differential conformance suite: [`introspect`] parses the container without decoding pixels
//! (structure, CICP, transforms, metadata payloads), and [`decode_rgba`] returns libavif's own
//! RGBA8 presentation of the primary image (its colour conversion and alpha merge), the oracle for
//! `gamut_avif::AvifImage::decode_primary_rgba8`.
//!
//! All `unsafe` FFI is confined here behind these safe entry points.

#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case)]

mod sys {
    #![allow(dead_code)]
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

/// A decoded planar image: one tightly packed `width * height` plane per component, each sample
/// widened to `u16` (8-bit samples occupy `0..=255`, 10-/12-bit use the wider range).
///
/// gamut emits 4:4:4 stills, so the three planes are full-resolution and carry `[Y, U, V]` (which
/// under the identity matrix are `G, B, R`).
pub struct DecodedImage {
    /// Luma width in pixels.
    pub width: u32,
    /// Luma height in pixels.
    pub height: u32,
    /// Bits per component (8, 10, or 12).
    pub bit_depth: u8,
    /// `[Y, U, V]` planes, each in raster order with no row padding; samples widened to `u16`.
    pub planes: [Vec<u16>; 3],
}

/// Decodes the first frame of an AVIF file into its 4:4:4 YUV planes (8/10/12-bit, widened to `u16`).
///
/// # Errors
///
/// Returns a message (including libavif's own result string) if the file cannot be parsed or
/// decoded, or if the decoded image is not 4:4:4 or not 8/10/12-bit (the forms gamut emits).
pub fn decode_avif(avif: &[u8]) -> Result<DecodedImage, String> {
    // SAFETY: the decoder and image handles below are created and destroyed in matched pairs on
    // every return path; pointers passed to libavif stay valid for each call's duration.
    unsafe { decode_inner(avif) }
}

unsafe fn decode_inner(avif: &[u8]) -> Result<DecodedImage, String> {
    unsafe {
        let decoder = sys::avifDecoderCreate();
        if decoder.is_null() {
            return Err("avifDecoderCreate returned null".into());
        }
        let image = sys::avifImageCreateEmpty();
        if image.is_null() {
            sys::avifDecoderDestroy(decoder);
            return Err("avifImageCreateEmpty returned null".into());
        }

        let result = sys::avifDecoderReadMemory(decoder, image, avif.as_ptr(), avif.len());
        let out = if result == sys::AVIF_RESULT_OK {
            extract(&*image)
        } else {
            Err(format!(
                "avifDecoderReadMemory failed: {}",
                result_str(result)
            ))
        };

        sys::avifImageDestroy(image);
        sys::avifDecoderDestroy(decoder);
        out
    }
}

/// Copies the three YUV planes out of a decoded `avifImage` into owned, unpadded buffers.
unsafe fn extract(image: &sys::avifImage) -> Result<DecodedImage, String> {
    let depth = image.depth as u8;
    if !matches!(depth, 8 | 10 | 12) {
        return Err(format!("unexpected bit depth: {depth}-bit"));
    }
    if image.yuvFormat != sys::AVIF_PIXEL_FORMAT_YUV444 {
        return Err(format!(
            "expected 4:4:4, got pixel format {}",
            image.yuvFormat
        ));
    }
    let w = image.width as usize;
    let h = image.height as usize;

    // SAFETY: a successfully decoded 4:4:4 image owns three planes of `h` rows; `yuvRowBytes[p]`
    // (the byte stride) spaces consecutive rows of plane `p`.
    unsafe {
        let mut planes = [Vec::new(), Vec::new(), Vec::new()];
        for (p, plane) in planes.iter_mut().enumerate() {
            let base = image.yuvPlanes[p];
            if base.is_null() {
                return Err(format!("plane {p} is null"));
            }
            *plane = copy_plane(base, image.yuvRowBytes[p] as usize, w, h, depth);
        }
        let [y, u, v] = planes;
        Ok(DecodedImage {
            width: image.width,
            height: image.height,
            bit_depth: depth,
            planes: [y, u, v],
        })
    }
}

/// Copies a `w`×`h` plane from a strided libavif buffer into a tightly packed `u16` `Vec`. `depth`
/// is the bit depth: at 8 the source samples are bytes (widened to `u16`); at 10/12 they are native-
/// endian `u16` and `byte_stride` is in bytes.
unsafe fn copy_plane(
    base: *const u8,
    byte_stride: usize,
    w: usize,
    h: usize,
    depth: u8,
) -> Vec<u16> {
    let mut out = vec![0u16; w * h];
    // SAFETY: caller guarantees `base` addresses `h` rows of at least `w` samples spaced
    // `byte_stride` bytes apart; each read stays within row `row`'s `w` samples and `out` is exactly
    // `w * h` elements.
    unsafe {
        for row in 0..h {
            let row_base = base.add(byte_stride * row);
            for col in 0..w {
                out[row * w + col] = if depth == 8 {
                    u16::from(*row_base.add(col))
                } else {
                    *row_base.cast::<u16>().add(col)
                };
            }
        }
    }
    out
}

/// The structural facts libavif reports for an AVIF file after `avifDecoderParse` — no pixel
/// decode involved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvifStructure {
    /// Primary image width in pixels.
    pub width: u32,
    /// Primary image height in pixels.
    pub height: u32,
    /// Bits per component.
    pub depth: u8,
    /// The raw `avifPixelFormat` value (1 = 4:4:4, 2 = 4:2:2, 3 = 4:2:0, 4 = monochrome).
    pub yuv_format: u32,
    /// Whether the YUV signal is full range.
    pub full_range: bool,
    /// CICP `colour_primaries`.
    pub color_primaries: u16,
    /// CICP `transfer_characteristics`.
    pub transfer_characteristics: u16,
    /// CICP `matrix_coefficients`.
    pub matrix_coefficients: u16,
    /// Whether an alpha plane (alpha auxiliary item) is present.
    pub alpha_present: bool,
    /// The `irot` angle (anti-clockwise quarter turns), when the transform is present.
    pub irot_angle: Option<u8>,
    /// The `imir` axis, when the transform is present.
    pub imir_axis: Option<u8>,
    /// The `clap` box values `[widthN, widthD, heightN, heightD, horizOffN, horizOffD, vertOffN,
    /// vertOffD]`, when the transform is present.
    pub clap: Option<[u32; 8]>,
    /// The ICC profile payload (empty if none).
    pub icc: Vec<u8>,
    /// The Exif payload as stored (empty if none). libavif strips the 4-byte
    /// `exif_tiff_header_offset` prefix, exposing the TIFF stream directly.
    pub exif: Vec<u8>,
    /// The XMP packet (empty if none).
    pub xmp: Vec<u8>,
}

/// Parses an AVIF file's container structure with libavif — `avifDecoderParse` only, no pixel
/// decode — and reports the primary image's structural facts.
///
/// # Errors
///
/// Returns a message (including libavif's own result string) if the container cannot be parsed.
pub fn introspect(avif: &[u8]) -> Result<AvifStructure, String> {
    // SAFETY: the decoder handle is created and destroyed in a matched pair on every return path;
    // the input pointer stays valid for the call's duration and libavif copies what it keeps.
    unsafe { introspect_inner(avif) }
}

unsafe fn introspect_inner(avif: &[u8]) -> Result<AvifStructure, String> {
    unsafe {
        let decoder = sys::avifDecoderCreate();
        if decoder.is_null() {
            return Err("avifDecoderCreate returned null".into());
        }
        let out = (|| {
            let result = sys::avifDecoderSetIOMemory(decoder, avif.as_ptr(), avif.len());
            if result != sys::AVIF_RESULT_OK {
                return Err(format!(
                    "avifDecoderSetIOMemory failed: {}",
                    result_str(result)
                ));
            }
            let result = sys::avifDecoderParse(decoder);
            if result != sys::AVIF_RESULT_OK {
                return Err(format!("avifDecoderParse failed: {}", result_str(result)));
            }
            let image = &*(*decoder).image;
            let flags = image.transformFlags;
            let clap = &image.clap;
            Ok(AvifStructure {
                width: image.width,
                height: image.height,
                depth: image.depth as u8,
                yuv_format: image.yuvFormat as u32,
                full_range: image.yuvRange == sys::AVIF_RANGE_FULL,
                color_primaries: image.colorPrimaries,
                transfer_characteristics: image.transferCharacteristics,
                matrix_coefficients: image.matrixCoefficients,
                alpha_present: (*decoder).alphaPresent != 0,
                irot_angle: (flags & sys::AVIF_TRANSFORM_IROT != 0).then_some(image.irot.angle),
                imir_axis: (flags & sys::AVIF_TRANSFORM_IMIR != 0).then_some(image.imir.axis),
                clap: (flags & sys::AVIF_TRANSFORM_CLAP != 0).then_some([
                    clap.widthN,
                    clap.widthD,
                    clap.heightN,
                    clap.heightD,
                    clap.horizOffN,
                    clap.horizOffD,
                    clap.vertOffN,
                    clap.vertOffD,
                ]),
                icc: rw_data(&image.icc),
                exif: rw_data(&image.exif),
                xmp: rw_data(&image.xmp),
            })
        })();
        sys::avifDecoderDestroy(decoder);
        out
    }
}

/// Copies an `avifRWData` payload into an owned `Vec` (empty for a null/zero-length payload).
unsafe fn rw_data(data: &sys::avifRWData) -> Vec<u8> {
    if data.data.is_null() || data.size == 0 {
        return Vec::new();
    }
    // SAFETY: libavif guarantees `data` addresses `size` bytes for the decoder's lifetime.
    unsafe { std::slice::from_raw_parts(data.data, data.size).to_vec() }
}

/// Decodes the primary image of an AVIF file to libavif's own interleaved RGBA8 presentation —
/// colour conversion (`avifImageYUVToRGB`) and alpha merge included, `irot`/`imir`/`clap` **not**
/// applied (libavif leaves transforms to the caller). Returns `(width, height, rgba)`.
///
/// # Errors
///
/// Returns a message (including libavif's own result string) if the file cannot be parsed,
/// decoded, or converted.
pub fn decode_rgba(avif: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    // SAFETY: decoder/image handles and the RGB pixel allocation are created and released in
    // matched pairs on every return path.
    unsafe { decode_rgba_inner(avif) }
}

unsafe fn decode_rgba_inner(avif: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    unsafe {
        let decoder = sys::avifDecoderCreate();
        if decoder.is_null() {
            return Err("avifDecoderCreate returned null".into());
        }
        let image = sys::avifImageCreateEmpty();
        if image.is_null() {
            sys::avifDecoderDestroy(decoder);
            return Err("avifImageCreateEmpty returned null".into());
        }

        let result = sys::avifDecoderReadMemory(decoder, image, avif.as_ptr(), avif.len());
        let out = if result == sys::AVIF_RESULT_OK {
            to_rgba(&*image)
        } else {
            Err(format!(
                "avifDecoderReadMemory failed: {}",
                result_str(result)
            ))
        };

        sys::avifImageDestroy(image);
        sys::avifDecoderDestroy(decoder);
        out
    }
}

/// Converts a decoded `avifImage` to tightly packed RGBA8 via `avifImageYUVToRGB`.
unsafe fn to_rgba(image: &sys::avifImage) -> Result<(u32, u32, Vec<u8>), String> {
    // SAFETY: `rgb` is initialized by `avifRGBImageSetDefaults`; its pixel buffer is allocated by
    // `avifRGBImageAllocatePixels` and freed on every path after the copy.
    unsafe {
        let mut rgb: sys::avifRGBImage = std::mem::zeroed();
        sys::avifRGBImageSetDefaults(&mut rgb, image);
        rgb.format = sys::AVIF_RGB_FORMAT_RGBA;
        rgb.depth = 8;
        // Nearest-neighbour chroma upsampling, matching gamut-avif's documented RGBA policy, so a
        // pixel diff against it isolates real conversion errors instead of resampler choice.
        rgb.chromaUpsampling = sys::AVIF_CHROMA_UPSAMPLING_NEAREST;
        let result = sys::avifRGBImageAllocatePixels(&mut rgb);
        if result != sys::AVIF_RESULT_OK {
            return Err(format!(
                "avifRGBImageAllocatePixels failed: {}",
                result_str(result)
            ));
        }
        let result = sys::avifImageYUVToRGB(image, &mut rgb);
        let out = if result == sys::AVIF_RESULT_OK {
            let (w, h) = (rgb.width as usize, rgb.height as usize);
            let mut pixels = vec![0u8; w * h * 4];
            for row in 0..h {
                let src = rgb.pixels.add(rgb.rowBytes as usize * row);
                std::ptr::copy_nonoverlapping(src, pixels[row * w * 4..].as_mut_ptr(), w * 4);
            }
            Ok((rgb.width, rgb.height, pixels))
        } else {
            Err(format!("avifImageYUVToRGB failed: {}", result_str(result)))
        };
        sys::avifRGBImageFreePixels(&mut rgb);
        out
    }
}

/// libavif's human-readable string for a result code.
unsafe fn result_str(result: sys::avifResult) -> String {
    unsafe {
        let ptr = sys::avifResultToString(result);
        if ptr.is_null() {
            return format!("avifResult({result})");
        }
        std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }
}
