//! Top-level: turn 4:4:4 identity planes into the AV1 temporal unit for an AVIF still image.

use crate::headers::{self, Av1StillConfig};
use crate::tile::FrameEncoder;
use gamut_color::Planar8;
use gamut_color::cicp::{ColourPrimaries, MatrixCoefficients, TransferCharacteristics};
use gamut_core::{Error, Result};

/// The encoded AV1 temporal unit (sequence-header OBU + frame OBU) for one still image, plus the
/// configuration values `gamut-avif` must mirror into the `av1C` and `colr` boxes.
#[derive(Debug, Clone)]
#[must_use]
pub struct EncodedStill {
    /// The OBU byte stream to place in the AVIF `mdat` item (no temporal delimiter).
    pub obus: Vec<u8>,
    /// Sequence-header field values for `av1C` / `colr`.
    pub config: Av1StillConfig,
}

/// The encoder's reconstructed image: exactly the samples a conformant decoder produces for
/// [`EncodedStill`]. Cropped to the display dimensions (the coded-grid padding is dropped, as on
/// decode). Used for the bit-exact decoder cross-check.
#[derive(Debug, Clone)]
#[must_use]
pub struct ReconImage {
    /// Display width.
    pub width: u32,
    /// Display height.
    pub height: u32,
    /// Reconstructed planes (Y=G, U=B, V=R), each `width * height` samples, row-major.
    pub planes: [Vec<u8>; 3],
}

/// Encodes 8-bit 4:4:4 identity planes (Y=G, U=B, V=R) as a lossless AV1 intra keyframe.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] for zero-sized images, or [`Error::Unsupported`] if the
/// dimensions exceed AV1 level 6.0 (out of M0 scope).
pub fn encode_still_lossless_identity(planes: &Planar8) -> Result<EncodedStill> {
    Ok(encode_still_intra(planes, 0)?.0)
}

/// Encodes 8-bit 4:4:4 identity planes (Y=G, U=B, V=R) as an AV1 intra keyframe at quantizer
/// `qindex` (`base_q_idx`). `qindex == 0` is lossless; `1..=255` is lossy intra (DCT +
/// quantization), selecting the coefficient-CDF quantizer context per spec §8.3.2 (0 if `qindex`
/// ≤ 20, 1 if ≤ 60, 2 if ≤ 120, else 3). Returns the encoded still and the reconstruction (the
/// exact decoder output) for verification.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] for zero-sized images, or [`Error::Unsupported`] if the
/// dimensions exceed AV1 level 6.0.
pub fn encode_still_intra(planes: &Planar8, qindex: u8) -> Result<(EncodedStill, ReconImage)> {
    let width = planes.width();
    let height = planes.height();
    if width == 0 || height == 0 {
        return Err(Error::InvalidInput("image has a zero dimension"));
    }

    let config = Av1StillConfig {
        seq_profile: 1,
        seq_level_idx_0: headers::pick_level(width, height)?,
        seq_tier_0: 0,
        high_bitdepth: false,
        twelve_bit: false,
        monochrome: false,
        chroma_subsampling_x: 0,
        chroma_subsampling_y: 0,
        chroma_sample_position: 0,
        color_primaries: ColourPrimaries::Bt709.code_point(),
        transfer_characteristics: TransferCharacteristics::Srgb.code_point(),
        matrix_coefficients: MatrixCoefficients::Identity.code_point(),
        full_range: true,
    };

    let mi_cols = 2 * ((width + 7) >> 3);
    let mi_rows = 2 * ((height + 7) >> 3);

    let seq_payload = headers::sequence_header_payload(&config, width, height, qindex > 0);
    let mut frame_payload = headers::frame_header_payload(width, height, mi_cols, mi_rows, qindex);
    let (tile_bytes, recon) = FrameEncoder::new(planes, qindex).encode();
    // tile_group_obu (§5.11.1): the frame header already emitted the tile-group prefix (the
    // `tile_start_and_end_present_flag` and re-alignment for a multi-tile frame). Each tile but the
    // last is prefixed by its byte size minus one as a little-endian `TileSizeBytes`-byte field.
    for (i, tile) in tile_bytes.iter().enumerate() {
        if i + 1 < tile_bytes.len() {
            let sz = (tile.len() - 1) as u32;
            frame_payload.extend_from_slice(&sz.to_le_bytes()[..headers::TILE_SIZE_BYTES]);
        }
        frame_payload.extend_from_slice(tile);
    }

    // Crop the reconstruction from the coded grid to the display dimensions. For the lossless path
    // the reconstruction equals the source, so use the source planes directly.
    let recon_planes = if qindex == 0 {
        [
            crop(planes.plane(0), width, planes.width(), height),
            crop(planes.plane(1), width, planes.width(), height),
            crop(planes.plane(2), width, planes.width(), height),
        ]
    } else {
        [
            crop(&recon.planes[0], width, recon.coded_w as u32, height),
            crop(&recon.planes[1], width, recon.coded_w as u32, height),
            crop(&recon.planes[2], width, recon.coded_w as u32, height),
        ]
    };

    let still = EncodedStill {
        obus: headers::assemble_temporal_unit(&seq_payload, &frame_payload),
        config,
    };
    let recon = ReconImage {
        width,
        height,
        planes: recon_planes,
    };
    Ok((still, recon))
}

/// Crops a `src_stride`-wide plane to `width × height`, row-major.
fn crop(plane: &[u8], width: u32, src_stride: u32, height: u32) -> Vec<u8> {
    let (w, sw, h) = (width as usize, src_stride as usize, height as usize);
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        out.extend_from_slice(&plane[y * sw..y * sw + w]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamut_color::Planar8;

    /// Builds identity planes from an RGB generator.
    fn planes(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) -> Planar8 {
        let mut rgb = vec![0u8; (w * h * 3) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 3) as usize;
                let p = f(x, y);
                rgb[i..i + 3].copy_from_slice(&p);
            }
        }
        Planar8::from_rgb8_identity(&rgb, w, h).unwrap()
    }

    /// Parses the low-overhead OBU stream into `(obu_type, payload_len)` pairs, asserting it tiles
    /// the buffer exactly.
    fn parse_obus(d: &[u8]) -> Vec<(u8, usize)> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < d.len() {
            let hb = d[i];
            i += 1;
            let obu_type = (hb >> 3) & 0xf;
            let has_size = (hb >> 1) & 1;
            assert_eq!(has_size, 1, "M0 always sets obu_has_size_field");
            // leb128 size
            let mut size = 0usize;
            let mut shift = 0;
            loop {
                let b = d[i];
                i += 1;
                size |= usize::from(b & 0x7f) << shift;
                shift += 7;
                if b & 0x80 == 0 {
                    break;
                }
            }
            out.push((obu_type, size));
            i += size;
        }
        assert_eq!(i, d.len(), "OBUs must tile the temporal unit exactly");
        out
    }

    #[test]
    fn obu_stream_is_seq_then_frame() {
        let p = planes(40, 24, |x, y| [(x * 3) as u8, (y * 5) as u8, (x + y) as u8]);
        let e = encode_still_lossless_identity(&p).unwrap();
        assert_eq!(e.config.seq_profile, 1);
        assert_eq!(e.config.matrix_coefficients, 0);
        assert_eq!(e.config.chroma_subsampling_x, 0);
        assert!(e.config.full_range);
        let obus = parse_obus(&e.obus);
        assert_eq!(obus.len(), 2);
        assert_eq!(obus[0].0, 1, "first OBU is the sequence header");
        assert_eq!(obus[1].0, 6, "second OBU is the frame");
    }

    #[test]
    fn deterministic_output() {
        let p = planes(33, 17, |x, y| [(x ^ y) as u8, (x * 7) as u8, (y * 3) as u8]);
        assert_eq!(
            encode_still_lossless_identity(&p).unwrap().obus,
            encode_still_lossless_identity(&p).unwrap().obus
        );
    }

    #[test]
    fn solid_color_uses_all_zero_path() {
        // A flat image makes every residual zero, exercising the txb_skip = all_zero branch.
        let e = encode_still_lossless_identity(&planes(64, 64, |_, _| [200, 100, 50])).unwrap();
        assert!(!e.obus.is_empty());
    }

    #[test]
    fn high_contrast_exercises_golomb() {
        // A ±max checkerboard produces large WHT coefficients (golomb tails).
        let e = encode_still_lossless_identity(&planes(48, 48, |x, y| {
            let v = if (x + y) % 2 == 0 { 0 } else { 255 };
            [v, 255 - v, v]
        }))
        .unwrap();
        assert!(!e.obus.is_empty());
    }

    #[test]
    fn assorted_sizes_encode() {
        // Edge sizes (padding + forced partition splits) and multi-superblock frames.
        for (w, h) in [
            (1, 1),
            (7, 3),
            (64, 1),
            (1, 64),
            (100, 80),
            (130, 70),
            (256, 256),
        ] {
            let p = planes(w, h, |x, y| [(x * 11) as u8, (y * 13) as u8, (x * y) as u8]);
            let e = encode_still_lossless_identity(&p).unwrap();
            assert_eq!(parse_obus(&e.obus).len(), 2);
        }
    }

    #[test]
    fn rejects_zero_dimension() {
        let p = Planar8::from_rgb8_identity(&[], 0, 0).unwrap();
        assert!(encode_still_lossless_identity(&p).is_err());
    }

    #[test]
    fn lossy_encode_structure_and_determinism() {
        // Exercises the lossy path (DCT + quant + reconstruction) across sizes/qindex without a
        // decoder: OBU framing, deterministic output, and the reconstruction dimensions. The
        // qindex set spans every coefficient-CDF quantizer context (≤20, ≤60, ≤120, else).
        for &q in &[1u8, 8, 20, 40, 90, 200, 255] {
            for (w, h) in [(1, 1), (8, 8), (17, 13), (40, 24), (130, 70)] {
                let p = planes(w, h, |x, y| {
                    [(x * 7 + y) as u8, (x ^ (y * 3)) as u8, (x + y * 5) as u8]
                });
                let (still, recon) = encode_still_intra(&p, q).unwrap();
                assert_eq!(parse_obus(&still.obus).len(), 2, "{w}x{h} q{q}");
                assert_eq!(recon.width, w);
                assert_eq!(recon.height, h);
                for plane in &recon.planes {
                    assert_eq!(plane.len(), (w * h) as usize);
                }
                // Determinism.
                let (again, _) = encode_still_intra(&p, q).unwrap();
                assert_eq!(still.obus, again.obus, "{w}x{h} q{q} not deterministic");
            }
        }
    }

    #[test]
    fn lossy_flat_image_reconstructs_near_source() {
        // A solid color quantizes every AC residual to zero; the DC-prediction reconstruction
        // should land within a couple of levels of the source (light quantization).
        let (_, recon) = encode_still_intra(&planes(48, 40, |_, _| [200, 100, 50]), 12).unwrap();
        // Planes are identity-mapped Y=G, U=B, V=R, so the source plane DCs are [100, 50, 200].
        for (plane, &want) in recon.planes.iter().zip(&[100u8, 50, 200]) {
            for &got in plane {
                assert!(
                    i32::from(got).abs_diff(i32::from(want)) <= 3,
                    "flat recon {got} far from {want}"
                );
            }
        }
    }

    #[test]
    fn lossy_high_contrast_encodes() {
        // A ±max checkerboard makes large coefficients (golomb tails) in the lossy path too.
        let (still, recon) = encode_still_intra(
            &planes(48, 48, |x, y| {
                let v = if (x + y) % 2 == 0 { 0 } else { 255 };
                [v, 255 - v, v]
            }),
            16,
        )
        .unwrap();
        assert_eq!(parse_obus(&still.obus).len(), 2);
        assert_eq!(recon.planes[0].len(), 48 * 48);
    }
}
