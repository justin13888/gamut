//! The DNG decoder: parse a DNG back to its raw image, colour profile, and version.
//!
//! This reverses [`crate::encoder`]. It walks the IFD tree (IFD 0 plus the raw sub-IFD reached
//! through `SubIFDs`), decompresses and unpacks the sensor samples, and reconstructs the
//! [`RawImage`] and [`CameraProfile`]. As stated in the crate docs, demosaicing and colour
//! rendering are out of scope — the decoder returns the sensor samples, not a viewable image.

use gamut_core::{Dimensions, Error, Result};
use gamut_ifd::{ByteOrder, Ifd, Value, Variant, read, read_ifd_at};

use crate::profile::CameraProfile;
use crate::raw::RawImage;
use crate::values::{
    CalibrationIlluminant, Compression, PhotometricInterpretation, ProfileEmbedPolicy,
};
use crate::{bitpack, compression, lossless_jpeg, tags};

/// A decoded DNG: the raw sensor image, the camera colour profile, and the declared DNG version.
#[derive(Debug, Clone)]
pub struct DecodedDng {
    /// The raw sensor image (CFA mosaic or linear), with its photometry and levels.
    pub raw: RawImage,
    /// The camera colour profile reconstructed from IFD 0.
    pub profile: CameraProfile,
    /// The `DNGVersion` the file declares.
    pub dng_version: [u8; 4],
}

/// Decoder for DNG (Adobe Digital Negative) raw images.
#[derive(Debug, Clone, Default)]
pub struct DngDecoder {
    _private: (),
}

impl DngDecoder {
    /// Creates a decoder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Decodes `data` (a DNG file) into its raw image, profile, and version.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if the container is malformed or a required tag is missing,
    /// or [`Error::Unsupported`] for a compression scheme or photometry not yet decodable.
    pub fn decode(&self, data: &[u8]) -> Result<DecodedDng> {
        let file = read(data)?;
        let order = file.order;
        let variant = file.variant;
        let ifd0 = file
            .ifds
            .first()
            .ok_or(Error::InvalidInput("DNG: file has no IFD 0"))?;

        let raw_ifd = find_raw_ifd(ifd0, data, order, variant)?;
        let raw = decode_raw_image(&raw_ifd, data, order)?;
        let profile = decode_profile(ifd0)?;
        let dng_version = read_version(ifd0)?;

        Ok(DecodedDng {
            raw,
            profile,
            dng_version,
        })
    }
}

/// Whether `ifd` holds a raw image (`PhotometricInterpretation` is CFA or LinearRaw).
fn is_raw_ifd(ifd: &Ifd) -> bool {
    matches!(
        ifd.get_u32(tags::PHOTOMETRIC_INTERPRETATION)
            .and_then(|c| u16::try_from(c).ok())
            .and_then(PhotometricInterpretation::from_code),
        Some(PhotometricInterpretation::Cfa | PhotometricInterpretation::LinearRaw)
    )
}

/// Locates the full-resolution raw IFD: a `SubIFDs` child with a raw photometry, or IFD 0 itself.
fn find_raw_ifd(ifd0: &Ifd, data: &[u8], order: ByteOrder, variant: Variant) -> Result<Ifd> {
    if let Some(offsets) = ifd0.get_u32_vec(tags::SUB_IFDS) {
        for offset in offsets {
            if let Ok(sub) = read_ifd_at(data, u64::from(offset), order, variant)
                && is_raw_ifd(&sub)
            {
                return Ok(sub);
            }
        }
    }
    if is_raw_ifd(ifd0) {
        return Ok(ifd0.clone());
    }
    Err(Error::InvalidInput("DNG: no raw image IFD found"))
}

/// Reconstructs the [`RawImage`] from a raw IFD and the file's strip data.
fn decode_raw_image(ifd: &Ifd, data: &[u8], order: ByteOrder) -> Result<RawImage> {
    let width = ifd
        .get_u32(tags::IMAGE_WIDTH)
        .ok_or(Error::InvalidInput("DNG: raw IFD missing ImageWidth"))?;
    let height = ifd
        .get_u32(tags::IMAGE_LENGTH)
        .ok_or(Error::InvalidInput("DNG: raw IFD missing ImageLength"))?;
    let spp = ifd.get_u32(tags::SAMPLES_PER_PIXEL).unwrap_or(1);
    let bits = ifd
        .get_u32_vec(tags::BITS_PER_SAMPLE)
        .and_then(|v| v.first().copied())
        .ok_or(Error::InvalidInput("DNG: raw IFD missing BitsPerSample"))? as u16;
    let compression = Compression::from_code(ifd.get_u32(tags::COMPRESSION).unwrap_or(1) as u16)
        .ok_or(Error::Unsupported("DNG: unknown compression"))?;
    let photometric = ifd
        .get_u32(tags::PHOTOMETRIC_INTERPRETATION)
        .and_then(|c| u16::try_from(c).ok())
        .and_then(PhotometricInterpretation::from_code)
        .ok_or(Error::InvalidInput(
            "DNG: raw IFD missing PhotometricInterpretation",
        ))?;

    let samples_per_row = (width as usize)
        .checked_mul(spp as usize)
        .ok_or(Error::InvalidInput("DNG: dimensions overflow"))?;
    let expected = samples_per_row
        .checked_mul(height as usize)
        .ok_or(Error::InvalidInput("DNG: dimensions overflow"))?;

    let chunks = strip_chunks(ifd, data)?;
    let samples = match compression {
        Compression::Uncompressed | Compression::Deflate => {
            let mut packed = Vec::new();
            for chunk in &chunks {
                packed.extend_from_slice(&compression::decompress(compression, chunk)?);
            }
            bitpack::unpack(&packed, bits, samples_per_row, height as usize, order)
        }
        // Lossless JPEG decodes samples directly; strips concatenate as row-bands.
        Compression::LosslessJpeg => {
            let mut samples = Vec::with_capacity(expected);
            let mut rows_seen = 0usize;
            for chunk in &chunks {
                let jpeg = lossless_jpeg::decode(chunk)?;
                if jpeg.width != width as usize || jpeg.components != spp as usize {
                    return Err(Error::InvalidInput("DNG: lossless-JPEG geometry mismatch"));
                }
                rows_seen += jpeg.height;
                samples.extend(jpeg.samples);
            }
            if rows_seen != height as usize {
                return Err(Error::InvalidInput("DNG: lossless-JPEG rows mismatch"));
            }
            samples
        }
        _ => {
            return Err(Error::Unsupported(
                "DNG: this compression is not yet decodable",
            ));
        }
    };
    if samples.len() != expected {
        return Err(Error::InvalidInput("DNG: raw image data is truncated"));
    }

    let dims = Dimensions::new(width, height)?;
    let mut raw = match photometric {
        PhotometricInterpretation::Cfa => {
            let dim = ifd
                .get_u32_vec(tags::CFA_REPEAT_PATTERN_DIM)
                .filter(|v| v.len() == 2)
                .ok_or(Error::InvalidInput("DNG: CFA missing CFARepeatPatternDim"))?;
            let pattern = bytes_value(ifd.get(tags::CFA_PATTERN))
                .ok_or(Error::InvalidInput("DNG: CFA missing CFAPattern"))?;
            let repeat = (dim[0] as u16, dim[1] as u16);
            let mut raw = RawImage::new_cfa(dims, bits, repeat, pattern, samples)?;
            if let Some(colors) = bytes_value(ifd.get(tags::CFA_PLANE_COLOR)) {
                raw = raw.with_cfa_plane_color(colors);
            }
            if let Some(layout) = ifd
                .get_u32(tags::CFA_LAYOUT)
                .and_then(|c| u16::try_from(c).ok())
                .and_then(crate::values::CfaLayout::from_code)
            {
                raw = raw.with_cfa_layout(layout);
            }
            raw
        }
        PhotometricInterpretation::LinearRaw => {
            RawImage::new_linear_raw(dims, bits, spp as u16, samples)?
        }
        _ => return Err(Error::Unsupported("DNG: photometry is not a raw image")),
    };

    if let Some(black) = ifd.get_u32(tags::BLACK_LEVEL) {
        raw = raw.with_black_level(black);
    }
    if let Some(white) = ifd.get_u32(tags::WHITE_LEVEL) {
        raw = raw.with_white_level(white);
    }
    if let Some(area) = ifd.get_u32_vec(tags::ACTIVE_AREA).filter(|v| v.len() == 4) {
        raw = raw.with_active_area([area[0], area[1], area[2], area[3]]);
    }
    if let (Some(origin), Some(size)) = (
        ifd.get_u32_vec(tags::DEFAULT_CROP_ORIGIN)
            .filter(|v| v.len() == 2),
        ifd.get_u32_vec(tags::DEFAULT_CROP_SIZE)
            .filter(|v| v.len() == 2),
    ) {
        raw = raw.with_default_crop([origin[0], origin[1]], [size[0], size[1]]);
    }
    Ok(raw)
}

/// Returns the IFD's strips as raw byte slices (in order), to be decompressed per the scheme.
fn strip_chunks<'a>(ifd: &Ifd, data: &'a [u8]) -> Result<Vec<&'a [u8]>> {
    let offsets = ifd
        .get_u32_vec(tags::STRIP_OFFSETS)
        .ok_or(Error::Unsupported(
            "DNG: only stripped raw is decodable so far (no tiles)",
        ))?;
    let counts = ifd
        .get_u32_vec(tags::STRIP_BYTE_COUNTS)
        .ok_or(Error::InvalidInput("DNG: missing StripByteCounts"))?;
    if offsets.len() != counts.len() {
        return Err(Error::InvalidInput(
            "DNG: strip offset/count length mismatch",
        ));
    }
    let mut chunks = Vec::with_capacity(offsets.len());
    for (offset, count) in offsets.iter().zip(counts) {
        let start = *offset as usize;
        let end = start
            .checked_add(count as usize)
            .ok_or(Error::InvalidInput("DNG: strip extent overflow"))?;
        chunks.push(
            data.get(start..end)
                .ok_or(Error::InvalidInput("DNG: strip out of bounds"))?,
        );
    }
    Ok(chunks)
}

/// Reconstructs the [`CameraProfile`] from IFD 0's identity and calibration tags.
fn decode_profile(ifd0: &Ifd) -> Result<CameraProfile> {
    let model = ascii_value(ifd0.get(tags::UNIQUE_CAMERA_MODEL))
        .ok_or(Error::InvalidInput("DNG: missing UniqueCameraModel"))?;
    let color_matrix1 = matrix9(ifd0, tags::COLOR_MATRIX1)?;
    let illuminant1 = illuminant(ifd0, tags::CALIBRATION_ILLUMINANT1)
        .ok_or(Error::InvalidInput("DNG: missing CalibrationIlluminant1"))?;
    let neutral = f64_vec(ifd0.get(tags::AS_SHOT_NEUTRAL))
        .filter(|v| v.len() == 3)
        .ok_or(Error::InvalidInput("DNG: missing AsShotNeutral"))?;

    let mut profile = CameraProfile::new(
        model,
        color_matrix1,
        illuminant1,
        [neutral[0], neutral[1], neutral[2]],
    )?;

    if let (Ok(matrix2), Some(illuminant2)) = (
        matrix9(ifd0, tags::COLOR_MATRIX2),
        illuminant(ifd0, tags::CALIBRATION_ILLUMINANT2),
    ) {
        profile = profile.with_second_illuminant(matrix2, illuminant2);
    }
    if let Ok(cc1) = matrix9(ifd0, tags::CAMERA_CALIBRATION1) {
        profile =
            profile.with_camera_calibration(cc1, matrix9(ifd0, tags::CAMERA_CALIBRATION2).ok());
    }
    if let Ok(fm1) = matrix9(ifd0, tags::FORWARD_MATRIX1) {
        profile = profile.with_forward_matrices(fm1, matrix9(ifd0, tags::FORWARD_MATRIX2).ok());
    }
    if let Some(ab) = f64_vec(ifd0.get(tags::ANALOG_BALANCE)).filter(|v| v.len() == 3) {
        profile = profile.with_analog_balance([ab[0], ab[1], ab[2]]);
    }
    if let Some(be) = f64_vec(ifd0.get(tags::BASELINE_EXPOSURE)).and_then(|v| v.first().copied()) {
        profile = profile.with_baseline_exposure(be);
    }
    if let Some(name) = ascii_value(ifd0.get(tags::PROFILE_NAME)) {
        profile = profile.with_profile_name(name);
    }
    if let Some(policy) = ifd0
        .get_u32(tags::PROFILE_EMBED_POLICY)
        .and_then(ProfileEmbedPolicy::from_code)
    {
        profile = profile.with_profile_embed_policy(policy);
    }
    Ok(profile)
}

/// Reads `DNGVersion` as a 4-byte array (defaulting trailing bytes to zero).
fn read_version(ifd0: &Ifd) -> Result<[u8; 4]> {
    let bytes = bytes_value(ifd0.get(tags::DNG_VERSION))
        .ok_or(Error::InvalidInput("DNG: missing DNGVersion"))?;
    let mut version = [0u8; 4];
    for (slot, b) in version.iter_mut().zip(bytes) {
        *slot = b;
    }
    Ok(version)
}

/// Extracts a `BYTE`/`UNDEFINED` value's bytes (or a `SHORT` array narrowed to bytes).
fn bytes_value(value: Option<&Value>) -> Option<Vec<u8>> {
    match value? {
        Value::Byte(v) | Value::Undefined(v) => Some(v.clone()),
        Value::Short(v) => Some(v.iter().map(|&x| x as u8).collect()),
        _ => None,
    }
}

/// Extracts an `ASCII` value.
fn ascii_value(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Ascii(s) => Some(s.clone()),
        _ => None,
    }
}

/// Converts a `RATIONAL`/`SRATIONAL` value to `f64`s.
fn f64_vec(value: Option<&Value>) -> Option<Vec<f64>> {
    let ratio = |n: f64, d: f64| if d == 0.0 { 0.0 } else { n / d };
    match value? {
        Value::Rational(r) => Some(r.iter().map(|&(n, d)| ratio(n.into(), d.into())).collect()),
        Value::SRational(r) => Some(r.iter().map(|&(n, d)| ratio(n.into(), d.into())).collect()),
        _ => None,
    }
}

/// Reads a 9-element `(S)RATIONAL` matrix tag as `[f64; 9]`.
fn matrix9(ifd: &Ifd, tag: u16) -> Result<[f64; 9]> {
    let v = f64_vec(ifd.get(tag))
        .filter(|v| v.len() == 9)
        .ok_or(Error::InvalidInput("DNG: expected a 3x3 matrix tag"))?;
    let mut m = [0.0; 9];
    m.copy_from_slice(&v);
    Ok(m)
}

/// Reads a `CalibrationIlluminant` tag.
fn illuminant(ifd: &Ifd, tag: u16) -> Option<CalibrationIlluminant> {
    ifd.get_u32(tag)
        .and_then(|c| u16::try_from(c).ok())
        .and_then(CalibrationIlluminant::from_code)
}
