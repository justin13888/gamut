//! The DNG decoder: parse a DNG back to its raw image, colour profile, and version.
//!
//! This reverses [`crate::encoder`]. It walks the IFD tree (IFD 0 plus the raw sub-IFD reached
//! through `SubIFDs`), decompresses and unpacks the sensor samples, and reconstructs the
//! [`RawImage`] and [`CameraProfile`]. As stated in the crate docs, demosaicing and colour
//! rendering are out of scope — the decoder returns the sensor samples, not a viewable image.

use gamut_core::{Dimensions, Error, Result};
use gamut_ifd::{ByteOrder, Ifd, TiffFile, Value, Variant, read, read_ifd_at};

use crate::levels::RawLevels;
use crate::metadata::{DngMetadata, ExifMetadata};
use crate::opcode::OpcodeList;
use crate::profile::CameraProfile;
use crate::raw::RawImage;
use crate::values::{
    CalibrationIlluminant, Compression, PhotometricInterpretation, ProfileEmbedPolicy, SampleFormat,
};
use crate::{bitpack, compression, lossless_jpeg, tags};

/// A decoded DNG: the raw sensor image, the camera colour profile, and the declared DNG version.
#[derive(Debug, Clone)]
pub struct DecodedDng {
    /// The raw sensor image (CFA mosaic or linear), with its photometry and levels.
    pub raw: RawImage,
    /// The camera colour profile reconstructed from IFD 0.
    pub profile: CameraProfile,
    /// The `DNGVersion` the file declares, as its four dotted version octets in order — e.g. DNG
    /// 1.7.1.0 is `[1, 7, 1, 0]`. Kept as four bytes (not a packed `u32`) so each component reads
    /// directly and byte order never enters into it.
    pub dng_version: [u8; 4],
    /// Embedded metadata (EXIF sub-IFD + XMP/IPTC/ICC blocks), reconstructed from IFD 0.
    pub metadata: DngMetadata,
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

        let raw_ifd = find_raw_ifd(&file, data)?;
        let raw = decode_raw_image(&raw_ifd, data, order)?;
        let profile = decode_profile(ifd0)?;
        let dng_version = read_version(ifd0)?;
        let metadata = decode_metadata(ifd0, data, order, variant);

        Ok(DecodedDng {
            raw,
            profile,
            dng_version,
            metadata,
        })
    }
}

/// Reconstructs embedded metadata from IFD 0: the XMP/IPTC/ICC blocks and the EXIF sub-IFD.
fn decode_metadata(ifd0: &Ifd, data: &[u8], order: ByteOrder, variant: Variant) -> DngMetadata {
    let exif = ifd0
        .get_u32(tags::EXIF_IFD)
        .and_then(|offset| read_ifd_at(data, u64::from(offset), order, variant).ok())
        .map(|exif_ifd| decode_exif(&exif_ifd))
        .unwrap_or_default();
    DngMetadata {
        exif,
        xmp: bytes_value(ifd0.get(tags::XMP)),
        iptc: bytes_value(ifd0.get(tags::IPTC_NAA)),
        icc: bytes_value(ifd0.get(tags::ICC_PROFILE)),
    }
}

/// Reads the common capture settings out of an EXIF sub-IFD.
fn decode_exif(exif: &Ifd) -> ExifMetadata {
    ExifMetadata {
        exposure_time: rational_pair(exif.get(tags::EXPOSURE_TIME)),
        f_number: rational_pair(exif.get(tags::F_NUMBER)),
        iso_speed: exif
            .get_u32(tags::ISO_SPEED_RATINGS)
            .and_then(|v| u16::try_from(v).ok()),
        date_time_original: ascii_value(exif.get(tags::DATE_TIME_ORIGINAL)),
        focal_length: rational_pair(exif.get(tags::FOCAL_LENGTH)),
    }
}

/// Extracts the first `(numerator, denominator)` of an unsigned `RATIONAL` value.
fn rational_pair(value: Option<&Value>) -> Option<(u32, u32)> {
    value?.as_rationals()?.first().copied()
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

/// An upper bound on the `SubIFDs` nesting depth the decoder follows, bounding hostile pointer
/// graphs; legitimate DNG trees (IFD 0 → raw / enhanced / mask sub-IFDs) are two levels.
const MAX_SUBIFD_DEPTH: usize = 8;

/// Collects every IFD in the file — the top-level chain plus, recursively, every `SubIFDs`
/// child — in encounter order. Lenient by design: an unreadable child is skipped rather than
/// failing the whole decode, while offset de-duplication and the depth cap terminate hostile
/// pointer cycles.
fn walk_ifds(file: &TiffFile, data: &[u8]) -> Vec<Ifd> {
    let mut out = Vec::new();
    let mut visited: Vec<u64> = Vec::new();
    for ifd in &file.ifds {
        collect_sub_ifds(
            ifd,
            data,
            file.order,
            file.variant,
            &mut visited,
            1,
            &mut out,
        );
    }
    out
}

/// Pushes `ifd` and recurses into its `SubIFDs` children (see [`walk_ifds`]).
fn collect_sub_ifds(
    ifd: &Ifd,
    data: &[u8],
    order: ByteOrder,
    variant: Variant,
    visited: &mut Vec<u64>,
    depth: usize,
    out: &mut Vec<Ifd>,
) {
    out.push(ifd.clone());
    if depth >= MAX_SUBIFD_DEPTH {
        return;
    }
    // Offsets are read at full width: a BigTIFF writes `SubIFDs` as `LONG8`, whose values only
    // matter past 4 GiB — exactly where a u32 reading would fail.
    let Some(offsets) = ifd.get_u64_vec(tags::SUB_IFDS) else {
        return;
    };
    for offset in offsets {
        if visited.contains(&offset) {
            continue;
        }
        visited.push(offset);
        if let Ok(sub) = read_ifd_at(data, offset, order, variant) {
            collect_sub_ifds(&sub, data, order, variant, visited, depth + 1, out);
        }
    }
}

/// Locates the full-resolution raw IFD anywhere in the IFD forest — the top-level chain plus all
/// nested `SubIFDs` (real DNGs keep the raw in a sub-IFD of IFD 0, but TIFF/EP permits either).
///
/// Prefers an IFD whose `NewSubFileType` is 0 (the main image; the tag defaults to 0 when
/// absent) with a raw photometry, falling back to any raw-photometry IFD.
fn find_raw_ifd(file: &TiffFile, data: &[u8]) -> Result<Ifd> {
    let mut fallback = None;
    for ifd in walk_ifds(file, data) {
        if !is_raw_ifd(&ifd) {
            continue;
        }
        if ifd.get_u32(tags::NEW_SUBFILE_TYPE).unwrap_or(0) == 0 {
            return Ok(ifd);
        }
        if fallback.is_none() {
            fallback = Some(ifd);
        }
    }
    fallback.ok_or(Error::InvalidInput("DNG: no raw image IFD found"))
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

    // SampleFormat (339) defaults to unsigned integer, the only encoding whose code values the
    // u16 sample model represents. Anything else must fail cleanly, not silently misdecode.
    if let Some(formats) = ifd.get_u32_vec(tags::SAMPLE_FORMAT) {
        for format in formats {
            match u16::try_from(format).ok().and_then(SampleFormat::from_code) {
                Some(SampleFormat::UnsignedInteger) => {}
                Some(SampleFormat::FloatingPoint) => {
                    return Err(Error::Unsupported(
                        "DNG: floating-point sample data is not supported",
                    ));
                }
                _ => {
                    return Err(Error::Unsupported(
                        "DNG: only unsigned-integer samples are supported",
                    ));
                }
            }
        }
    }

    let samples_per_row = (width as usize)
        .checked_mul(spp as usize)
        .ok_or(Error::InvalidInput("DNG: dimensions overflow"))?;
    let expected = samples_per_row
        .checked_mul(height as usize)
        .ok_or(Error::InvalidInput("DNG: dimensions overflow"))?;

    // Each strip is an independent sample stream: `RowsPerStrip` rows (fewer for the last), with
    // sub-byte depths MSB-packed to byte-aligned rows *per strip*. Decoding therefore works
    // strip by strip — concatenating the packed bytes first would misalign any strip whose
    // packed length is not what whole-image geometry predicts.
    let rows_per_strip = match ifd.get_u32(tags::ROWS_PER_STRIP) {
        Some(0) => {
            return Err(Error::InvalidInput("DNG: RowsPerStrip must be non-zero"));
        }
        Some(r) => r as usize,
        None => height as usize,
    };

    // Reject an undecodable scheme up front, so an empty strip list cannot mask it.
    if !matches!(
        compression,
        Compression::Uncompressed | Compression::Deflate | Compression::LosslessJpeg
    ) {
        return Err(Error::Unsupported(
            "DNG: this compression is not yet decodable",
        ));
    }

    let chunks = strip_chunks(ifd, data)?;
    let mut samples = Vec::with_capacity(expected);
    let mut remaining_rows = height as usize;
    for chunk in &chunks {
        let rows = rows_per_strip.min(remaining_rows);
        if rows == 0 {
            return Err(Error::InvalidInput("DNG: more strips than image rows"));
        }
        let want = rows * samples_per_row;
        match compression {
            Compression::Uncompressed | Compression::Deflate => {
                let bytes = compression::decompress(compression, chunk)?;
                let mut got = bitpack::unpack(&bytes, bits, samples_per_row, rows, order);
                if got.len() < want {
                    return Err(Error::InvalidInput("DNG: raw image data is truncated"));
                }
                got.truncate(want); // tolerate strip padding, per TIFF practice
                samples.extend(got);
            }
            // Lossless JPEG decodes samples directly; strips concatenate as row-bands. The JPEG
            // stream's internal width/height/components need not match the strip's geometry —
            // only the total sample count must (DNG 1.7.1, "Compression": real CFA writers store
            // a two-component stream at half width).
            Compression::LosslessJpeg => {
                let jpeg = lossless_jpeg::decode(chunk)?;
                if jpeg.samples.len() != want {
                    return Err(Error::InvalidInput(
                        "DNG: lossless-JPEG sample count mismatch",
                    ));
                }
                samples.extend(jpeg.samples);
            }
            _ => {
                return Err(Error::Unsupported(
                    "DNG: this compression is not yet decodable",
                ));
            }
        }
        remaining_rows -= rows;
    }
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

    let active_area = ifd
        .get_u32_vec(tags::ACTIVE_AREA)
        .filter(|v| v.len() == 4)
        .map(|v| [v[0], v[1], v[2], v[3]]);
    if let Some(area) = active_area {
        raw = raw.with_active_area(area);
    }
    if let (Some(origin), Some(size)) = (
        ifd.get_u32_vec(tags::DEFAULT_CROP_ORIGIN)
            .filter(|v| v.len() == 2),
        ifd.get_u32_vec(tags::DEFAULT_CROP_SIZE)
            .filter(|v| v.len() == 2),
    ) {
        raw = raw.with_default_crop([origin[0], origin[1]], [size[0], size[1]]);
    }
    raw = raw.with_levels(decode_levels(ifd, spp as u16, bits, dims, active_area)?)?;
    if let Some(areas) = decode_masked_areas(ifd)? {
        raw = raw.with_masked_areas(areas);
    }
    if let Some(list) = decode_opcode_list(ifd, tags::OPCODE_LIST1)? {
        raw = raw.with_opcode_list1(list);
    }
    if let Some(list) = decode_opcode_list(ifd, tags::OPCODE_LIST2)? {
        raw = raw.with_opcode_list2(list);
    }
    if let Some(list) = decode_opcode_list(ifd, tags::OPCODE_LIST3)? {
        raw = raw.with_opcode_list3(list);
    }
    Ok(raw)
}

/// Reads an `OpcodeList1/2/3` tag (UNDEFINED bytes) into a typed [`OpcodeList`]. The container
/// is big-endian regardless of the file's byte order (DNG 1.7.1 p. 105); a malformed container
/// is an error, not silently dropped.
fn decode_opcode_list(ifd: &Ifd, tag: u16) -> Result<Option<OpcodeList>> {
    let Some(value) = ifd.get(tag) else {
        return Ok(None);
    };
    let bytes = value.as_bytes().ok_or(Error::InvalidInput(
        "DNG: opcode lists must be UNDEFINED byte data",
    ))?;
    Ok(Some(OpcodeList::parse(bytes)?))
}

/// Reads the level family — `BlackLevelRepeatDim`/`BlackLevel` (+ the `DeltaH`/`DeltaV`
/// refinements) and the per-plane `WhiteLevel` — from a raw IFD (DNG 1.7.1 pp. 27–29).
///
/// A single-value `BlackLevel`/`WhiteLevel` broadcasts to every cell/plane (common writer
/// shorthand, and what pre-pattern gamut-dng emitted); any other count mismatch is an error.
/// Delta counts must match the active area (defaulting to the full image), mirroring the DNG SDK.
fn decode_levels(
    ifd: &Ifd,
    spp: u16,
    bits: u16,
    dims: Dimensions,
    active_area: Option<[u32; 4]>,
) -> Result<RawLevels> {
    let repeat = match ifd.get_u32_vec(tags::BLACK_LEVEL_REPEAT_DIM) {
        Some(v) if v.len() == 2 => {
            let rows = u16::try_from(v[0]).ok().filter(|r| *r > 0);
            let cols = u16::try_from(v[1]).ok().filter(|c| *c > 0);
            match (rows, cols) {
                (Some(rows), Some(cols)) => (rows, cols),
                _ => {
                    return Err(Error::InvalidInput(
                        "DNG: BlackLevelRepeatDim dimensions must be non-zero",
                    ));
                }
            }
        }
        Some(_) => {
            return Err(Error::InvalidInput(
                "DNG: BlackLevelRepeatDim needs two values (rows, cols)",
            ));
        }
        None => (1, 1),
    };

    let cells = usize::from(repeat.0) * usize::from(repeat.1) * usize::from(spp);
    let black = match ifd.get(tags::BLACK_LEVEL) {
        None => vec![0.0; cells],
        Some(value) => {
            let v = unsigned_f64s(value).ok_or(Error::InvalidInput(
                "DNG: BlackLevel must be SHORT, LONG, or RATIONAL",
            ))?;
            if v.len() == cells {
                v
            } else if v.len() == 1 {
                vec![v[0]; cells]
            } else {
                return Err(Error::InvalidInput(
                    "DNG: BlackLevel count must be repeat rows * cols * samples per pixel",
                ));
            }
        }
    };

    let white = match ifd.get_u32_vec(tags::WHITE_LEVEL) {
        None => vec![f64::from((1u32 << bits) - 1); usize::from(spp)],
        Some(v) if v.len() == usize::from(spp) => v.into_iter().map(f64::from).collect(),
        Some(v) if v.len() == 1 => vec![f64::from(v[0]); usize::from(spp)],
        Some(_) => {
            return Err(Error::InvalidInput(
                "DNG: WhiteLevel needs one value per sample plane",
            ));
        }
    };

    let mut levels = RawLevels::new(spp, repeat, black, white)?;

    if let Some(value) = ifd.get(tags::LINEARIZATION_TABLE) {
        let Value::Short(table) = value else {
            return Err(Error::InvalidInput("DNG: LinearizationTable must be SHORT"));
        };
        if table.is_empty() {
            return Err(Error::InvalidInput(
                "DNG: LinearizationTable must not be empty",
            ));
        }
        levels = levels.with_linearization_table(table.clone());
    }

    let (aa_width, aa_height) = active_area_size(dims, active_area);
    if let Some(deltas) = decode_deltas(ifd, tags::BLACK_LEVEL_DELTA_H, aa_width, "column")? {
        levels = levels.with_black_delta_h(deltas);
    }
    if let Some(deltas) = decode_deltas(ifd, tags::BLACK_LEVEL_DELTA_V, aa_height, "row")? {
        levels = levels.with_black_delta_v(deltas);
    }
    Ok(levels)
}

/// Reads a `BlackLevelDeltaH`/`BlackLevelDeltaV` tag, requiring `expected` values (one per
/// active-area column/row — the SDK enforces the same).
fn decode_deltas(
    ifd: &Ifd,
    tag: u16,
    expected: usize,
    axis: &'static str,
) -> Result<Option<Vec<f64>>> {
    let Some(value) = ifd.get(tag) else {
        return Ok(None);
    };
    let deltas: Vec<f64> = value
        .as_srationals()
        .ok_or(Error::InvalidInput(
            "DNG: black-level deltas must be SRATIONAL",
        ))?
        .iter()
        .map(|&(n, d)| ratio(f64::from(n), f64::from(d)))
        .collect();
    if deltas.len() != expected {
        return Err(Error::InvalidInput(match axis {
            "column" => "DNG: BlackLevelDeltaH needs one value per active-area column",
            _ => "DNG: BlackLevelDeltaV needs one value per active-area row",
        }));
    }
    Ok(Some(deltas))
}

/// The active-area `(width, height)`, defaulting to the full image when the tag is absent.
fn active_area_size(dims: Dimensions, active_area: Option<[u32; 4]>) -> (usize, usize) {
    match active_area {
        Some([top, left, bottom, right]) => (
            right.saturating_sub(left) as usize,
            bottom.saturating_sub(top) as usize,
        ),
        None => (dims.width as usize, dims.height as usize),
    }
}

/// Reads `MaskedAreas` as `[top, left, bottom, right]` rectangles (count must be a positive
/// multiple of four, DNG 1.7.1 p. 44).
fn decode_masked_areas(ifd: &Ifd) -> Result<Option<Vec<[u32; 4]>>> {
    let Some(flat) = ifd.get_u32_vec(tags::MASKED_AREAS) else {
        return Ok(None);
    };
    if flat.is_empty() || flat.len() % 4 != 0 {
        return Err(Error::InvalidInput(
            "DNG: MaskedAreas count must be a positive multiple of four",
        ));
    }
    Ok(Some(
        flat.chunks_exact(4)
            .map(|r| [r[0], r[1], r[2], r[3]])
            .collect(),
    ))
}

/// Returns the IFD's strips as raw byte slices (in order), to be decompressed per the scheme.
/// Offsets and counts are read at full 64-bit width (BigTIFF writes them as `LONG8`).
fn strip_chunks<'a>(ifd: &Ifd, data: &'a [u8]) -> Result<Vec<&'a [u8]>> {
    let offsets = ifd
        .get_u64_vec(tags::STRIP_OFFSETS)
        .ok_or(Error::Unsupported(
            "DNG: only stripped raw is decodable so far (no tiles)",
        ))?;
    let counts = ifd
        .get_u64_vec(tags::STRIP_BYTE_COUNTS)
        .ok_or(Error::InvalidInput("DNG: missing StripByteCounts"))?;
    byte_chunks(&offsets, &counts, data)
}

/// Resolves parallel offset/byte-count arrays into in-bounds byte slices.
fn byte_chunks<'a>(offsets: &[u64], counts: &[u64], data: &'a [u8]) -> Result<Vec<&'a [u8]>> {
    if offsets.len() != counts.len() {
        return Err(Error::InvalidInput(
            "DNG: image-data offset/count length mismatch",
        ));
    }
    let mut chunks = Vec::with_capacity(offsets.len());
    for (&offset, &count) in offsets.iter().zip(counts) {
        let start = usize::try_from(offset)
            .map_err(|_| Error::InvalidInput("DNG: image data out of bounds"))?;
        let end = count
            .try_into()
            .ok()
            .and_then(|count: usize| start.checked_add(count))
            .ok_or(Error::InvalidInput("DNG: image-data extent overflow"))?;
        chunks.push(
            data.get(start..end)
                .ok_or(Error::InvalidInput("DNG: image data out of bounds"))?,
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

/// Extracts a `BYTE`/`UNDEFINED` value's bytes (or a `SHORT` array narrowed to bytes, used for
/// SHORT-typed `CFAPattern` variants — a DNG-specific reading `Value::as_bytes` rightly refuses).
fn bytes_value(value: Option<&Value>) -> Option<Vec<u8>> {
    let value = value?;
    if let Some(b) = value.as_bytes() {
        return Some(b.to_vec());
    }
    match value {
        Value::Short(v) => Some(v.iter().map(|&x| x as u8).collect()),
        _ => None,
    }
}

/// Extracts an `ASCII` (or Exif 3.0 `UTF8`) string value.
fn ascii_value(value: Option<&Value>) -> Option<String> {
    value?.as_str().map(ToOwned::to_owned)
}

/// Divides a rational's parts, mapping a zero denominator to `0.0` (the numeric policy is this
/// codec's, not the container's).
fn ratio(n: f64, d: f64) -> f64 {
    if d == 0.0 { 0.0 } else { n / d }
}

/// Converts a `RATIONAL`/`SRATIONAL` value to `f64`s.
fn f64_vec(value: Option<&Value>) -> Option<Vec<f64>> {
    let value = value?;
    if let Some(r) = value.as_rationals() {
        return Some(r.iter().map(|&(n, d)| ratio(n.into(), d.into())).collect());
    }
    value
        .as_srationals()
        .map(|r| r.iter().map(|&(n, d)| ratio(n.into(), d.into())).collect())
}

/// Converts an unsigned numeric value — `SHORT`, `LONG`, or `RATIONAL`, the three types the
/// `BlackLevel` tag allows (DNG 1.7.1 p. 28) — to `f64`s.
fn unsigned_f64s(value: &Value) -> Option<Vec<f64>> {
    match value {
        Value::Short(v) => Some(v.iter().map(|&x| f64::from(x)).collect()),
        Value::Long(v) => Some(v.iter().map(|&x| f64::from(x)).collect()),
        Value::Rational(r) => Some(r.iter().map(|&(n, d)| ratio(n.into(), d.into())).collect()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_raw_ifd_only_for_raw_photometry() {
        let mut ifd = Ifd::new();
        // An RGB preview IFD (PhotometricInterpretation = 2) is not a raw image.
        ifd.set(tags::PHOTOMETRIC_INTERPRETATION, Value::Short(vec![2]));
        assert!(!is_raw_ifd(&ifd));
        // A CFA IFD (32803) is.
        ifd.set(tags::PHOTOMETRIC_INTERPRETATION, Value::Short(vec![32803]));
        assert!(is_raw_ifd(&ifd));
        // A missing tag is not raw either.
        assert!(!is_raw_ifd(&Ifd::new()));
    }

    #[test]
    fn bytes_value_extracts_and_narrows() {
        assert_eq!(
            bytes_value(Some(&Value::Byte(vec![1, 2, 3]))),
            Some(vec![1, 2, 3])
        );
        // A SHORT array is narrowed to bytes (used for SHORT-typed CFAPattern variants).
        assert_eq!(
            bytes_value(Some(&Value::Short(vec![0x12, 0x34]))),
            Some(vec![0x12, 0x34])
        );
        assert_eq!(bytes_value(Some(&Value::Long(vec![1]))), None);
        assert_eq!(bytes_value(None), None);
    }

    #[test]
    fn decode_raw_image_rejects_lossless_jpeg_sample_count_mismatch() {
        // A 4x2, single-component lossless-JPEG strip (8 samples)...
        let samples: Vec<u16> = (0..8).map(|i| i as u16).collect();
        let jpeg = lossless_jpeg::encode(&samples, 4, 2, 1, 12).expect("encode");
        // ...described by an IFD that claims a 2x2 image (4 samples). Per spec only the *total
        // sample count* must match the strip — the stream's internal width/components are free
        // (real CFA writers halve the width and double the components) — so this fails on the
        // count, not on any width/components equality.
        let mut ifd = Ifd::new();
        ifd.set(tags::IMAGE_WIDTH, Value::Short(vec![2]));
        ifd.set(tags::IMAGE_LENGTH, Value::Short(vec![2]));
        ifd.set(tags::SAMPLES_PER_PIXEL, Value::Short(vec![1]));
        ifd.set(tags::BITS_PER_SAMPLE, Value::Short(vec![12]));
        ifd.set(tags::COMPRESSION, Value::Short(vec![7])); // lossless JPEG
        ifd.set(tags::PHOTOMETRIC_INTERPRETATION, Value::Short(vec![32803])); // CFA
        ifd.set(tags::STRIP_OFFSETS, Value::Long(vec![0]));
        ifd.set(
            tags::STRIP_BYTE_COUNTS,
            Value::Long(vec![jpeg.len() as u32]),
        );

        let err = decode_raw_image(&ifd, &jpeg, ByteOrder::LittleEndian).unwrap_err();
        assert!(
            matches!(err, Error::InvalidInput(m) if m.contains("sample count")),
            "expected a sample-count mismatch error, got {err:?}"
        );
    }

    /// A reshaped lossless-JPEG stream — half width, doubled components, same total sample
    /// count — decodes, per the spec's total-count rule (a width/components equality would
    /// wrongly reject it).
    #[test]
    fn decode_raw_image_accepts_reshaped_lossless_jpeg_stream() {
        // 8 samples encoded as a 2-wide, 2-component, 2-row stream for a 4x2 one-plane image.
        let samples: Vec<u16> = (0..8).map(|i| i as u16).collect();
        let jpeg = lossless_jpeg::encode(&samples, 2, 2, 2, 12).expect("encode");
        let mut ifd = Ifd::new();
        ifd.set(tags::IMAGE_WIDTH, Value::Short(vec![4]));
        ifd.set(tags::IMAGE_LENGTH, Value::Short(vec![2]));
        ifd.set(tags::SAMPLES_PER_PIXEL, Value::Short(vec![1]));
        ifd.set(tags::BITS_PER_SAMPLE, Value::Short(vec![12]));
        ifd.set(tags::COMPRESSION, Value::Short(vec![7]));
        ifd.set(tags::PHOTOMETRIC_INTERPRETATION, Value::Short(vec![32803]));
        ifd.set(tags::CFA_REPEAT_PATTERN_DIM, Value::Short(vec![2, 2]));
        ifd.set(tags::CFA_PATTERN, Value::Byte(vec![0, 1, 1, 2]));
        ifd.set(tags::STRIP_OFFSETS, Value::Long(vec![0]));
        ifd.set(
            tags::STRIP_BYTE_COUNTS,
            Value::Long(vec![jpeg.len() as u32]),
        );

        let raw = decode_raw_image(&ifd, &jpeg, ByteOrder::LittleEndian).expect("decode");
        assert_eq!(raw.samples(), &samples[..]);
    }

    #[test]
    fn decode_raw_image_rejects_non_unsigned_sample_formats() {
        let mut ifd = Ifd::new();
        ifd.set(tags::IMAGE_WIDTH, Value::Short(vec![2]));
        ifd.set(tags::IMAGE_LENGTH, Value::Short(vec![2]));
        ifd.set(tags::SAMPLES_PER_PIXEL, Value::Short(vec![1]));
        ifd.set(tags::BITS_PER_SAMPLE, Value::Short(vec![16]));
        ifd.set(tags::COMPRESSION, Value::Short(vec![1]));
        ifd.set(tags::PHOTOMETRIC_INTERPRETATION, Value::Short(vec![32803]));
        ifd.set(tags::STRIP_OFFSETS, Value::Long(vec![0]));
        ifd.set(tags::STRIP_BYTE_COUNTS, Value::Long(vec![8]));

        // Float data is a distinct, named rejection (a real, deferred DNG feature)...
        ifd.set(tags::SAMPLE_FORMAT, Value::Short(vec![3]));
        let err = decode_raw_image(&ifd, &[0; 8], ByteOrder::LittleEndian).unwrap_err();
        assert!(
            matches!(err, Error::Unsupported(m) if m.contains("floating-point")),
            "expected a floating-point rejection, got {err:?}"
        );
        // ...signed and unrecognised formats fail generically...
        ifd.set(tags::SAMPLE_FORMAT, Value::Short(vec![2]));
        assert!(decode_raw_image(&ifd, &[0; 8], ByteOrder::LittleEndian).is_err());
        ifd.set(tags::SAMPLE_FORMAT, Value::Short(vec![9]));
        assert!(decode_raw_image(&ifd, &[0; 8], ByteOrder::LittleEndian).is_err());
        // ...and the explicit unsigned default decodes.
        ifd.set(tags::SAMPLE_FORMAT, Value::Short(vec![1]));
        ifd.set(tags::CFA_REPEAT_PATTERN_DIM, Value::Short(vec![2, 2]));
        ifd.set(tags::CFA_PATTERN, Value::Byte(vec![0, 1, 1, 2]));
        assert!(decode_raw_image(&ifd, &[0; 8], ByteOrder::LittleEndian).is_ok());
    }
}
