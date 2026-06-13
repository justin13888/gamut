//! The DNG encoder.

use gamut_core::{Error, Result};
use gamut_ifd::{ByteOrder, Ifd, Value, Variant};

use crate::profile::{CameraProfile, srational, urational};
use crate::raw::{RawImage, RawPhotometry};
use crate::values::{Compression, PhotometricInterpretation};
use crate::writer::{ImageBlocks, write_cfa_dng};
use crate::{preview, tags};

/// Encoder for DNG (Adobe Digital Negative) raw images.
///
/// [`encode`](Self::encode) writes a raw image — a CFA mosaic or a demosaiced `LinearRaw` — as a
/// DNG: an IFD 0 holding a small RGB preview plus the camera/colour-profile tags, and a raw
/// sub-IFD holding the full-resolution image. Defaults to little-endian (`II`) classic TIFF;
/// richer compression and metadata are added in later phases (see `STATUS.md`).
#[derive(Debug, Clone)]
pub struct DngEncoder {
    order: ByteOrder,
    dng_version: [u8; 4],
    backward_version: [u8; 4],
}

impl Default for DngEncoder {
    fn default() -> Self {
        Self {
            order: ByteOrder::LittleEndian,
            // 1.4.0.0 covers the baseline feature set; the backward version (oldest reader that can
            // parse the file) is the widely-supported 1.1.0.0.
            dng_version: [1, 4, 0, 0],
            backward_version: [1, 1, 0, 0],
        }
    }
}

impl DngEncoder {
    /// Creates an encoder that writes little-endian (`II`) DNG.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a copy of this encoder that writes in the given byte order.
    #[must_use]
    pub fn with_byte_order(mut self, order: ByteOrder) -> Self {
        self.order = order;
        self
    }

    /// Returns a copy of this encoder that declares the given `DNGVersion` (e.g. `[1, 4, 0, 0]`).
    #[must_use]
    pub fn with_dng_version(mut self, version: [u8; 4]) -> Self {
        self.dng_version = version;
        self
    }

    /// Returns a copy of this encoder that declares the given `DNGBackwardVersion` — the oldest DNG
    /// version a reader needs to fully parse the file.
    #[must_use]
    pub fn with_backward_version(mut self, version: [u8; 4]) -> Self {
        self.backward_version = version;
        self
    }

    /// The container variant this encoder writes.
    fn variant(&self) -> Variant {
        Variant::Classic
    }

    /// Encodes a raw image — a CFA mosaic or a demosaiced `LinearRaw` — as a DNG, appending the
    /// bytes to `out` and returning the number written.
    ///
    /// `raw` supplies the sensor samples plus the photometry and levels; `profile` supplies the
    /// colour calibration and as-shot white balance. The output is an IFD 0 holding an RGB preview
    /// plus the DNG/profile tags, with the full-resolution image in a `SubIFDs` sub-IFD.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] if the raw is not 3-colour (the profile is a `3 × 3` matrix)
    /// or its bit depth is not 8 or 16 (other depths need the bit-packing phase). Propagates
    /// buffer/validation errors.
    pub fn encode(
        &self,
        raw: &RawImage,
        profile: &CameraProfile,
        out: &mut Vec<u8>,
    ) -> Result<usize> {
        if color_plane_count(raw) != 3 {
            return Err(Error::Unsupported(
                "DNG: only 3-colour (RGB) raw images are supported so far",
            ));
        }
        let bits = raw.bits_per_sample();
        if bits != 8 && bits != 16 {
            return Err(Error::Unsupported(
                "DNG: only 8- and 16-bit samples are supported so far",
            ));
        }

        let (preview_dims, preview_rgb) = preview::raw_preview(raw);
        let ifd0 = self.build_ifd0(profile, preview_dims);
        let raw_ifd = build_raw_ifd(raw);

        let preview_blocks = ImageBlocks {
            offset_tag: tags::STRIP_OFFSETS,
            bytecount_tag: tags::STRIP_BYTE_COUNTS,
            blocks: vec![preview_rgb],
        };
        let raw_blocks = ImageBlocks {
            offset_tag: tags::STRIP_OFFSETS,
            bytecount_tag: tags::STRIP_BYTE_COUNTS,
            blocks: vec![serialize_samples(raw.samples(), bits, self.order)],
        };

        let bytes = write_cfa_dng(
            self.order,
            self.variant(),
            ifd0,
            &preview_blocks,
            raw_ifd,
            &raw_blocks,
        );
        out.extend_from_slice(&bytes);
        Ok(bytes.len())
    }

    /// Builds IFD 0: the RGB preview's image tags plus the DNG version, camera identity, and the
    /// colour-calibration profile. The `SubIFDs` pointer and strip offsets are filled in by the
    /// writer.
    fn build_ifd0(&self, profile: &CameraProfile, preview_dims: gamut_core::Dimensions) -> Ifd {
        let mut ifd = Ifd::new();
        // Preview image (a reduced-resolution RGB thumbnail).
        ifd.set(tags::NEW_SUBFILE_TYPE, Value::Long(vec![1]));
        ifd.set(tags::IMAGE_WIDTH, count_value(preview_dims.width));
        ifd.set(tags::IMAGE_LENGTH, count_value(preview_dims.height));
        ifd.set(tags::BITS_PER_SAMPLE, Value::Short(vec![8, 8, 8]));
        ifd.set(
            tags::COMPRESSION,
            Value::Short(vec![Compression::Uncompressed.code()]),
        );
        ifd.set(
            tags::PHOTOMETRIC_INTERPRETATION,
            Value::Short(vec![PhotometricInterpretation::Rgb.code()]),
        );
        ifd.set(tags::ORIENTATION, Value::Short(vec![1]));
        ifd.set(tags::SAMPLES_PER_PIXEL, Value::Short(vec![3]));
        ifd.set(tags::ROWS_PER_STRIP, count_value(preview_dims.height));
        ifd.set(tags::X_RESOLUTION, Value::Rational(vec![(72, 1)]));
        ifd.set(tags::Y_RESOLUTION, Value::Rational(vec![(72, 1)]));
        ifd.set(tags::RESOLUTION_UNIT, Value::Short(vec![2])); // inch
        ifd.set(tags::SOFTWARE, Value::Ascii("gamut-dng".to_owned()));
        ifd.set(
            tags::MODEL,
            Value::Ascii(profile.unique_camera_model().to_owned()),
        );

        // DNG identity + colour profile.
        ifd.set(tags::DNG_VERSION, Value::Byte(self.dng_version.to_vec()));
        ifd.set(
            tags::DNG_BACKWARD_VERSION,
            Value::Byte(self.backward_version.to_vec()),
        );
        ifd.set(
            tags::UNIQUE_CAMERA_MODEL,
            Value::Ascii(profile.unique_camera_model().to_owned()),
        );
        ifd.set(
            tags::COLOR_MATRIX1,
            Value::SRational(
                profile
                    .color_matrix1()
                    .iter()
                    .map(|&x| srational(x))
                    .collect(),
            ),
        );
        ifd.set(
            tags::CALIBRATION_ILLUMINANT1,
            Value::Short(vec![profile.calibration_illuminant1().code()]),
        );
        ifd.set(
            tags::AS_SHOT_NEUTRAL,
            Value::Rational(
                profile
                    .as_shot_neutral()
                    .iter()
                    .map(|&x| urational(x))
                    .collect(),
            ),
        );
        ifd
    }
}

/// The number of distinct colour planes a raw's photometry carries (`CFAPlaneColor` length for a
/// mosaic, the plane count for a linear image).
fn color_plane_count(raw: &RawImage) -> usize {
    match raw.photometry() {
        RawPhotometry::Cfa { plane_color, .. } => plane_color.len(),
        RawPhotometry::LinearRaw { planes } => usize::from(*planes),
    }
}

/// Builds the raw sub-IFD: the image-data tags, the photometry-specific tags (CFA pattern, or
/// `LinearRaw` planes), and the black/white levels. The strip offsets are filled in by the writer.
fn build_raw_ifd(raw: &RawImage) -> Ifd {
    let mut ifd = Ifd::new();
    let dims = raw.dimensions();
    let spp = raw.samples_per_pixel();
    ifd.set(tags::NEW_SUBFILE_TYPE, Value::Long(vec![0])); // full-resolution main image
    ifd.set(tags::IMAGE_WIDTH, count_value(dims.width));
    ifd.set(tags::IMAGE_LENGTH, count_value(dims.height));
    ifd.set(
        tags::BITS_PER_SAMPLE,
        Value::Short(vec![raw.bits_per_sample(); usize::from(spp)]),
    );
    ifd.set(
        tags::COMPRESSION,
        Value::Short(vec![Compression::Uncompressed.code()]),
    );
    ifd.set(tags::SAMPLES_PER_PIXEL, Value::Short(vec![spp]));
    ifd.set(tags::ROWS_PER_STRIP, count_value(dims.height));
    ifd.set(tags::SAMPLE_FORMAT, Value::Short(vec![1; usize::from(spp)])); // unsigned integer
    match raw.photometry() {
        RawPhotometry::Cfa {
            repeat,
            pattern,
            plane_color,
            layout,
        } => {
            ifd.set(
                tags::PHOTOMETRIC_INTERPRETATION,
                Value::Short(vec![PhotometricInterpretation::Cfa.code()]),
            );
            ifd.set(
                tags::CFA_REPEAT_PATTERN_DIM,
                Value::Short(vec![repeat.0, repeat.1]),
            );
            ifd.set(tags::CFA_PATTERN, Value::Byte(pattern.clone()));
            ifd.set(tags::CFA_PLANE_COLOR, Value::Byte(plane_color.clone()));
            ifd.set(tags::CFA_LAYOUT, Value::Short(vec![layout.code()]));
        }
        RawPhotometry::LinearRaw { .. } => {
            ifd.set(
                tags::PHOTOMETRIC_INTERPRETATION,
                Value::Short(vec![PhotometricInterpretation::LinearRaw.code()]),
            );
        }
    }
    ifd.set(tags::BLACK_LEVEL_REPEAT_DIM, Value::Short(vec![1, 1]));
    ifd.set(tags::BLACK_LEVEL, count_value(raw.black_level()));
    ifd.set(tags::WHITE_LEVEL, count_value(raw.white_level()));
    if let Some([t, l, b, r]) = raw.active_area() {
        ifd.set(tags::ACTIVE_AREA, Value::Long(vec![t, l, b, r]));
    }
    ifd
}

/// Serialises samples to bytes at `bits` per sample in `order` (caller guarantees 8 or 16).
fn serialize_samples(samples: &[u16], bits: u16, order: ByteOrder) -> Vec<u8> {
    if bits == 8 {
        samples.iter().map(|&s| s as u8).collect()
    } else {
        let mut out = Vec::with_capacity(samples.len() * 2);
        for &s in samples {
            out.extend_from_slice(&order.pack_u16(s));
        }
        out
    }
}

/// Stores a dimension/count as `SHORT` when it fits, else `LONG` (both valid per TIFF 6.0 §2).
fn count_value(n: u32) -> Value {
    if n <= u32::from(u16::MAX) {
        Value::Short(vec![n as u16])
    } else {
        Value::Long(vec![n])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::cfa_color;
    use crate::values::CalibrationIlluminant;
    use gamut_core::Dimensions;
    use gamut_ifd::read_ifd_at;

    fn sample_profile() -> CameraProfile {
        // A plausible XYZ->camera matrix and white balance; values are illustrative.
        let m = [0.95, -0.20, -0.05, -0.40, 1.30, 0.10, 0.02, -0.18, 0.85];
        CameraProfile::new(
            "gamut TestCam",
            m,
            CalibrationIlluminant::D65,
            [0.52, 1.0, 0.66],
        )
        .unwrap()
    }

    fn sample_raw(w: u32, h: u32, bits: u16) -> RawImage {
        let pattern = vec![
            cfa_color::RED,
            cfa_color::GREEN,
            cfa_color::GREEN,
            cfa_color::BLUE,
        ];
        let n = (w * h) as usize;
        let max = ((1u32 << bits) - 1) as u16;
        let samples: Vec<u16> = (0..n)
            .map(|i| ((i as u32 * 37) % u32::from(max)) as u16)
            .collect();
        RawImage::new_cfa(
            Dimensions::new(w, h).unwrap(),
            bits,
            (2, 2),
            pattern,
            samples,
        )
        .unwrap()
        .with_black_level(8)
        .with_white_level(u32::from(max))
        .with_active_area([0, 0, h, w])
    }

    fn deserialize_samples(bytes: &[u8], bits: u16, order: ByteOrder) -> Vec<u16> {
        if bits == 8 {
            bytes.iter().map(|&b| u16::from(b)).collect()
        } else {
            bytes
                .chunks_exact(2)
                .map(|c| order.u16([c[0], c[1]]))
                .collect()
        }
    }

    fn roundtrip_structure(order: ByteOrder, bits: u16) {
        let raw = sample_raw(8, 6, bits);
        let profile = sample_profile();
        let mut out = Vec::new();
        let n = DngEncoder::new()
            .with_byte_order(order)
            .encode(&raw, &profile, &mut out)
            .expect("encode");
        assert_eq!(n, out.len());

        // The container parses as a TIFF, IFD 0 is the preview + DNG/profile tags.
        let file = gamut_ifd::read(&out).expect("parse DNG");
        assert_eq!(file.order, order);
        assert_eq!(file.ifds.len(), 1, "raw lives in a sub-IFD, not the chain");
        let ifd0 = &file.ifds[0];
        assert_eq!(
            ifd0.get(tags::DNG_VERSION),
            Some(&Value::Byte(vec![1, 4, 0, 0]))
        );
        assert_eq!(
            ifd0.get(tags::UNIQUE_CAMERA_MODEL),
            Some(&Value::Ascii("gamut TestCam".to_owned()))
        );
        assert_eq!(ifd0.get_u32(tags::PHOTOMETRIC_INTERPRETATION), Some(2));
        assert_eq!(ifd0.get_u32(tags::CALIBRATION_ILLUMINANT1), Some(21)); // D65
        if let Some(Value::SRational(m)) = ifd0.get(tags::COLOR_MATRIX1) {
            assert_eq!(m.len(), 9);
            assert!((f64::from(m[0].0) / f64::from(m[0].1) - 0.95).abs() < 1e-4);
        } else {
            panic!("ColorMatrix1 missing/wrong type");
        }

        // Follow the SubIFDs pointer to the raw CFA image.
        let raw_off = ifd0.get_u32(tags::SUB_IFDS).expect("SubIFDs pointer");
        let raw_ifd = read_ifd_at(&out, raw_off.into(), order, Variant::Classic).expect("raw IFD");
        assert_eq!(raw_ifd.get_u32(tags::NEW_SUBFILE_TYPE), Some(0));
        assert_eq!(
            raw_ifd.get_u32(tags::PHOTOMETRIC_INTERPRETATION),
            Some(32803)
        );
        assert_eq!(raw_ifd.get_u32(tags::IMAGE_WIDTH), Some(8));
        assert_eq!(raw_ifd.get_u32(tags::IMAGE_LENGTH), Some(6));
        assert_eq!(
            raw_ifd.get_u32(tags::BITS_PER_SAMPLE),
            Some(u32::from(bits))
        );
        assert_eq!(
            raw_ifd.get(tags::CFA_PATTERN),
            Some(&Value::Byte(vec![0, 1, 1, 2]))
        );
        assert_eq!(raw_ifd.get_u32(tags::WHITE_LEVEL), Some(raw.white_level()));
        assert_eq!(raw_ifd.get_u32(tags::BLACK_LEVEL), Some(8));

        // The raw strip bytes deserialize back to the original mosaic.
        let off = raw_ifd.get_u32_vec(tags::STRIP_OFFSETS).expect("offsets")[0] as usize;
        let len = raw_ifd
            .get_u32_vec(tags::STRIP_BYTE_COUNTS)
            .expect("counts")[0] as usize;
        let got = deserialize_samples(&out[off..off + len], bits, order);
        assert_eq!(got, raw.samples(), "raw samples must round-trip");
    }

    #[test]
    fn cfa_dng_roundtrips_structure_le_16bit() {
        roundtrip_structure(ByteOrder::LittleEndian, 16);
    }

    #[test]
    fn cfa_dng_roundtrips_structure_be_16bit() {
        roundtrip_structure(ByteOrder::BigEndian, 16);
    }

    #[test]
    fn cfa_dng_roundtrips_structure_8bit() {
        roundtrip_structure(ByteOrder::LittleEndian, 8);
    }

    #[test]
    fn rejects_unsupported_inputs() {
        let profile = sample_profile();
        // 12-bit packing is a later phase.
        let raw12 = sample_raw(4, 4, 12);
        let mut out = Vec::new();
        assert!(
            DngEncoder::new()
                .encode(&raw12, &profile, &mut out)
                .is_err()
        );
    }
}
