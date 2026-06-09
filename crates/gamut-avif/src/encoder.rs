//! The AVIF still-image encoder: RGB → identity planes → AV1 temporal unit → ISOBMFF container.

use gamut_av1::{EncodedStill, encode_still_intra, encode_still_lossless_identity};
use gamut_color::Planar8;
use gamut_core::{Dimensions, Encoder, Result};
use gamut_isobmff::{Av1cConfig, AvifStillImage, NclxColr, write_avif_still};

/// Encodes images to AVIF still images.
///
/// 8-bit RGB in, mapped to AV1 identity-matrix 4:4:4. By default the encode is **lossless**;
/// [`AvifEncoder::with_qindex`] selects a lossy quantizer (`base_q_idx`, `1..=255`). Use
/// [`AvifEncoder::encode_rgb8`] for the explicit RGB entry point, or the [`Encoder`] trait (which
/// assumes the same 8-bit interleaved RGB layout).
#[derive(Debug, Clone, Copy, Default)]
pub struct AvifEncoder {
    /// AV1 `base_q_idx`: `0` is lossless, `1..=255` is lossy intra (higher = more quantization).
    qindex: u8,
}

impl AvifEncoder {
    /// Creates an encoder (lossless by default).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the AV1 quantizer (`base_q_idx`): `0` is lossless, `1..=255` is lossy intra (higher
    /// quantizes more aggressively for smaller files). Returns the updated encoder for chaining.
    #[must_use]
    pub fn with_qindex(mut self, qindex: u8) -> Self {
        self.qindex = qindex;
        self
    }

    /// Encodes 8-bit interleaved RGB (`width * height * 3` bytes, row-major, no padding) into a
    /// complete AVIF file appended to `out`, returning the number of bytes written. Lossless unless
    /// a lossy quantizer was set with [`AvifEncoder::with_qindex`].
    ///
    /// # Errors
    ///
    /// [`gamut_core::Error::InvalidInput`] if `rgb.len() != width * height * 3`, or
    /// [`gamut_core::Error::Unsupported`] if the dimensions exceed AV1 level 6.0.
    pub fn encode_rgb8(&self, rgb: &[u8], dims: Dimensions, out: &mut Vec<u8>) -> Result<usize> {
        let planes = Planar8::from_rgb8_identity(rgb, dims.width, dims.height)?;
        let still = if self.qindex == 0 {
            encode_still_lossless_identity(&planes)?
        } else {
            encode_still_intra(&planes, self.qindex)?.0
        };
        let file = build_avif(&still, dims);
        out.extend_from_slice(&file);
        Ok(file.len())
    }
}

/// Wraps the encoded AV1 temporal unit in the AVIF container, stamping `av1C`/`colr`/`ispe`/`pixi`
/// from the AV1 configuration so the cross-box consistency requirements hold by construction
/// (AVIF v1.2.0 §2.2, AV1-ISOBMFF v1.3.0 §2.3.4).
fn build_avif(still: &EncodedStill, dims: Dimensions) -> Vec<u8> {
    let c = &still.config;
    let av1c = Av1cConfig {
        seq_profile: c.seq_profile,
        seq_level_idx_0: c.seq_level_idx_0,
        seq_tier_0: c.seq_tier_0,
        high_bitdepth: c.high_bitdepth,
        twelve_bit: c.twelve_bit,
        monochrome: c.monochrome,
        chroma_subsampling_x: c.chroma_subsampling_x,
        chroma_subsampling_y: c.chroma_subsampling_y,
        chroma_sample_position: c.chroma_sample_position,
    };
    let nclx = NclxColr {
        colour_primaries: c.color_primaries,
        transfer_characteristics: c.transfer_characteristics,
        matrix_coefficients: c.matrix_coefficients,
        full_range: c.full_range,
    };
    let image = AvifStillImage {
        width: dims.width,
        height: dims.height,
        bit_depth: 8,
        num_channels: 3,
        av1c,
        nclx,
        item_data: &still.obus,
    };
    write_avif_still(&image)
}

impl Encoder for AvifEncoder {
    /// Encodes `pixels` as 8-bit interleaved RGB. See [`AvifEncoder::encode_rgb8`].
    fn encode(&self, pixels: &[u8], dims: Dimensions, out: &mut Vec<u8>) -> Result<usize> {
        self.encode_rgb8(pixels, dims, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(w: u32, h: u32) -> Vec<u8> {
        let mut rgb = vec![0u8; (w * h * 3) as usize];
        for (i, b) in rgb.iter_mut().enumerate() {
            *b = (i * 37) as u8;
        }
        let mut out = Vec::new();
        AvifEncoder::new()
            .encode_rgb8(
                &rgb,
                Dimensions {
                    width: w,
                    height: h,
                },
                &mut out,
            )
            .unwrap();
        out
    }

    #[test]
    fn produces_valid_avif_container() {
        let f = encode(40, 24);
        assert_eq!(&f[4..8], b"ftyp");
        for fourcc in [
            b"meta", b"av1C", b"ispe", b"pixi", b"colr", b"mdat", b"av01",
        ] {
            assert!(f.windows(4).any(|w| w == fourcc), "missing box {fourcc:?}");
        }
    }

    #[test]
    fn lossy_produces_valid_avif_container() {
        // A lossy quantizer still produces the same well-formed container; only the mdat payload
        // (the AV1 OBUs) differs. Exercises the with_qindex path across quantizer contexts.
        let mut rgb = vec![0u8; 48 * 32 * 3];
        for (i, b) in rgb.iter_mut().enumerate() {
            *b = (i * 29) as u8;
        }
        for q in [4u8, 40, 200] {
            let mut out = Vec::new();
            let n = AvifEncoder::new()
                .with_qindex(q)
                .encode_rgb8(
                    &rgb,
                    Dimensions {
                        width: 48,
                        height: 32,
                    },
                    &mut out,
                )
                .unwrap();
            assert_eq!(n, out.len());
            assert_eq!(&out[4..8], b"ftyp");
            for fourcc in [b"meta", b"av1C", b"ispe", b"mdat", b"av01"] {
                assert!(
                    out.windows(4).any(|w| w == fourcc),
                    "missing box {fourcc:?}"
                );
            }
        }
    }

    #[test]
    fn ispe_matches_dimensions() {
        let (w, h) = (37u32, 19u32);
        let f = encode(w, h);
        let pos = f.windows(4).position(|x| x == b"ispe").unwrap();
        let body = pos + 4 + 4; // skip 'ispe' fourcc + FullBox version/flags
        let rw = u32::from_be_bytes([f[body], f[body + 1], f[body + 2], f[body + 3]]);
        let rh = u32::from_be_bytes([f[body + 4], f[body + 5], f[body + 6], f[body + 7]]);
        assert_eq!((rw, rh), (w, h));
    }

    #[test]
    fn encoder_trait_matches_rgb8() {
        let rgb: Vec<u8> = (0..8 * 8 * 3u32).map(|i| (i * 3) as u8).collect();
        let d = Dimensions {
            width: 8,
            height: 8,
        };
        let mut via_rgb = Vec::new();
        AvifEncoder::new()
            .encode_rgb8(&rgb, d, &mut via_rgb)
            .unwrap();
        let mut via_trait = Vec::new();
        let n = AvifEncoder::new().encode(&rgb, d, &mut via_trait).unwrap();
        assert_eq!(via_rgb, via_trait);
        assert_eq!(n, via_trait.len());
    }

    #[test]
    fn rejects_wrong_length() {
        let mut out = Vec::new();
        let r = AvifEncoder::new().encode_rgb8(
            &[0; 10],
            Dimensions {
                width: 4,
                height: 4,
            },
            &mut out,
        );
        assert!(r.is_err());
    }

    #[test]
    fn appends_without_clobbering() {
        let mut out = vec![0xAA, 0xBB];
        let rgb = vec![128u8; 4 * 4 * 3];
        let n = AvifEncoder::new()
            .encode_rgb8(
                &rgb,
                Dimensions {
                    width: 4,
                    height: 4,
                },
                &mut out,
            )
            .unwrap();
        assert_eq!(out.len(), 2 + n);
        assert_eq!(&out[0..2], &[0xAA, 0xBB]);
    }
}
