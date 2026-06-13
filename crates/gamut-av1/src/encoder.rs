//! Top-level: turn 4:4:4 identity planes into the AV1 temporal unit for an AVIF still image.

use crate::headers::{self, Av1StillConfig};
use crate::tile::FrameEncoder;
use gamut_color::cicp::{ColourPrimaries, MatrixCoefficients, TransferCharacteristics};
use gamut_color::{BitDepth, Planar8};
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
    /// Bits per sample; the planes carry values in `0..=(1 << bit_depth.bits()) - 1`.
    pub bit_depth: BitDepth,
    /// Reconstructed planes (Y=G, U=B, V=R), each `width * height` samples, row-major, widened to
    /// `u16`.
    pub planes: [Vec<u16>; 3],
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
    encode_with(planes, qindex, None)
}

/// Like [`encode_still_intra`] but codes the frame with horizontal **superres** (§7.16): the source
/// is downscaled to `FrameWidth = (UpscaledWidth*8 + denom/2)/denom` (where `denom = coded_denom + 9`,
/// `coded_denom` in `0..=7`), coded at that width, and the reconstruction is upscaled back to the
/// display width. Lossy path only.
pub fn encode_still_intra_superres(
    planes: &Planar8,
    qindex: u8,
    coded_denom: u8,
) -> Result<(EncodedStill, ReconImage)> {
    encode_with(planes, qindex, Some(coded_denom))
}

fn encode_with(
    planes: &Planar8,
    qindex: u8,
    coded_denom: Option<u8>,
) -> Result<(EncodedStill, ReconImage)> {
    let width = planes.width();
    let height = planes.height();
    if width == 0 || height == 0 {
        return Err(Error::InvalidInput("image has a zero dimension"));
    }

    // Superres downscales the source horizontally to the coded (Frame) width; the reconstruction is
    // upscaled back to `width` at the end. `coded_src` is what the block encoder actually codes.
    let (coded_w, coded_src) = match coded_denom {
        Some(cd) => {
            let denom = cd as usize + 9;
            let dw = crate::filter::superres_downscaled_width(width as usize, denom);
            let dp: [Vec<u8>; 3] = std::array::from_fn(|i| {
                crate::filter::superres_downscale_plane(
                    planes.plane(i),
                    width as usize,
                    dw,
                    height as usize,
                )
            });
            (dw as u32, Planar8::from_planes(dw as u32, height, dp)?)
        }
        None => (width, planes.clone()),
    };

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

    let mi_cols = 2 * ((coded_w + 7) >> 3);
    let mi_rows = 2 * ((height + 7) >> 3);

    let seq_payload =
        headers::sequence_header_payload(&config, width, height, qindex > 0, coded_denom.is_some());
    let mut frame_payload =
        headers::frame_header_payload(coded_w, height, mi_cols, mi_rows, qindex, coded_denom);
    let (tile_bytes, recon) = FrameEncoder::new(&coded_src, qindex).encode();
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
    // the reconstruction equals the source. With superres the coded grid is the downscaled width, so
    // each plane is cropped to `coded_w` and then upscaled horizontally to the display `width`.
    let (uw, uh) = (width as usize, height as usize);
    let recon_planes: [Vec<u16>; 3] = if qindex == 0 {
        // Lossless: the reconstruction equals the 8-bit source; widen it into the u16 recon buffer.
        std::array::from_fn(|i| {
            crop(planes.plane(i), width, planes.width(), height)
                .into_iter()
                .map(u16::from)
                .collect()
        })
    } else if coded_denom.is_some() {
        // §7.4 order: superres upscale (downscaled → display width) happens **before** loop
        // restoration, which then runs on the upscaled luma. The deblocked-luma boundary is upscaled
        // too (only read by multi-stripe frames).
        let mut up: [Vec<u16>; 3] = std::array::from_fn(|i| {
            crate::filter::superres_upscale_plane(
                &recon.planes[i],
                recon.coded_w,
                coded_w as usize,
                uw,
                uh,
            )
        });
        let deblock_up = crate::filter::superres_upscale_plane(
            &recon.deblocked_luma,
            recon.coded_w,
            coded_w as usize,
            uw,
            uh,
        );
        crate::filter::loop_restore_wiener_luma(
            &mut up[0],
            &deblock_up,
            uw,
            uw,
            uh,
            crate::filter::WIENER_DEFAULT,
            crate::filter::WIENER_DEFAULT,
        );
        up
    } else {
        // No superres: loop restoration runs on the (display-width) coded reconstruction.
        let mut planes = recon.planes.clone();
        crate::filter::loop_restore_wiener_luma(
            &mut planes[0],
            &recon.deblocked_luma,
            recon.coded_w,
            uw,
            uh,
            crate::filter::WIENER_DEFAULT,
            crate::filter::WIENER_DEFAULT,
        );
        std::array::from_fn(|i| crop(&planes[i], width, recon.coded_w as u32, height))
    };

    let still = EncodedStill {
        obus: headers::assemble_temporal_unit(&seq_payload, &frame_payload),
        config,
    };
    let recon = ReconImage {
        width,
        height,
        bit_depth: BitDepth::from_bits(recon.bit_depth).ok_or(Error::Unsupported(
            "AV1: unsupported reconstruction bit depth",
        ))?,
        planes: recon_planes,
    };
    Ok((still, recon))
}

/// Crops a `src_stride`-wide plane to `width × height`, row-major.
fn crop<T: Copy>(plane: &[T], width: u32, src_stride: u32, height: u32) -> Vec<T> {
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

    /// Returns the payload bytes of the first OBU (the sequence header) — its header byte and
    /// LEB128 size prefix stripped.
    fn seq_header_payload(d: &[u8]) -> &[u8] {
        let mut i = 1; // skip the OBU header byte
        let (mut size, mut shift) = (0usize, 0);
        loop {
            let b = d[i];
            i += 1;
            size |= usize::from(b & 0x7f) << shift;
            shift += 7;
            if b & 0x80 == 0 {
                break;
            }
        }
        &d[i..i + size]
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
        // Each axis is rejected independently: a 0×0, a 0×4 and a 4×0 image must all fail with the
        // guard's own message. The mixed cases (exactly one zero) are what force the guard to be an
        // `||` — under `&&` a 0×4 slips through to fault deeper in the encoder.
        for (w, h) in [(0, 0), (0, 4), (4, 0)] {
            let p = Planar8::from_rgb8_identity(&[], w, h).unwrap();
            match encode_still_intra(&p, 0) {
                Err(Error::InvalidInput(msg)) => {
                    assert_eq!(msg, "image has a zero dimension", "{w}x{h}");
                }
                other => panic!("{w}x{h}: expected zero-dimension InvalidInput, got {other:?}"),
            }
        }
    }

    #[test]
    fn lossless_sequence_header_clears_lossy_flags() {
        // `encode_with` passes `qindex > 0` as the sequence header's `lossy` flag, which gates
        // enable_filter_intra/cdef/restoration. At qindex 0 (lossless) those bits must be 0; at a
        // lossy qindex they are 1. Same image and no superres, so the sequence-header payload differs
        // *only* by that flag — distinguishing `qindex > 0` from an always-true `qindex >= 0`.
        let p = planes(40, 24, |x, y| [(x * 3) as u8, (y * 5) as u8, (x + y) as u8]);
        let (lossless, _) = encode_still_intra(&p, 0).unwrap();
        let (lossy, _) = encode_still_intra(&p, 8).unwrap();
        assert_ne!(
            seq_header_payload(&lossless.obus),
            seq_header_payload(&lossy.obus),
            "lossless and lossy sequence headers must differ in the lossy flags",
        );
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
