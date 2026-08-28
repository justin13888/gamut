//! Top-level: turn 4:4:4 identity planes into the AV1 temporal unit for an AVIF still image.

use gamut_color::cicp::ColorRange;
use gamut_color::{BitDepth, ChromaSubsampling, Planar8};
use gamut_core::{Error, Result};

use crate::headers::{self, Av1Colour, Av1StillConfig};
use crate::tile::FrameEncoder;

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
#[non_exhaustive]
pub struct ReconImage {
    /// Display width.
    pub width: u32,
    /// Display height.
    pub height: u32,
    /// Bits per sample; the planes carry values in `0..=(1 << bit_depth.bits()) - 1`.
    pub bit_depth: BitDepth,
    /// Chroma subsampling of the reconstructed planes, which sizes the two chroma planes.
    pub subsampling: ChromaSubsampling,
    /// Reconstructed planes (Y=G, U=B, V=R), row-major and widened to `u16`. Each plane is its
    /// own [`plane_dimensions`](Self::plane_dimensions) — `width * height` for luma, and the
    /// subsampled extent for chroma.
    pub planes: [Vec<u16>; 3],
}

impl ReconImage {
    /// The dimensions of plane `index`: the display dimensions for luma, and
    /// [`ChromaSubsampling::chroma_dimensions`] for the two chroma planes.
    ///
    /// # Panics
    ///
    /// Panics if `index >= 3`, like slice indexing.
    #[must_use]
    pub fn plane_dimensions(&self, index: usize) -> (u32, u32) {
        match index {
            0 => (self.width, self.height),
            1 | 2 => self.subsampling.chroma_dimensions(self.width, self.height),
            _ => panic!("plane index {index} out of range (0..3)"),
        }
    }
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
    encode_with(planes, qindex, None, Av1Colour::default())
}

/// Like [`encode_still_intra`] but signals `colour` instead of the default identity/full-range
/// configuration.
///
/// The planes must already be in the layout `colour.matrix` describes — GBR for
/// [`MatrixCoefficients::Identity`], `Y/Cb/Cr` for a luma–chroma matrix (see
/// [`gamut_color::Planar8::from_rgb8_matrix`]). The coding tools are matrix-agnostic; `colour` only
/// selects what the sequence header — and, through [`EncodedStill::config`], the container's `av1C`
/// and `colr` boxes — declare the samples to be.
///
/// # Errors
///
/// As [`encode_still_intra`].
pub fn encode_still_intra_with(
    planes: &Planar8,
    qindex: u8,
    colour: Av1Colour,
) -> Result<(EncodedStill, ReconImage)> {
    encode_with(planes, qindex, None, colour)
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
    encode_with(planes, qindex, Some(coded_denom), Av1Colour::default())
}

fn encode_with(
    planes: &Planar8,
    qindex: u8,
    coded_denom: Option<u8>,
    colour: Av1Colour,
) -> Result<(EncodedStill, ReconImage)> {
    let width = planes.width();
    let height = planes.height();
    if width == 0 || height == 0 {
        return Err(Error::invalid_input(
            env!("CARGO_PKG_NAME"),
            "image has a zero dimension",
        ));
    }
    // The plane geometry is per-plane throughout, but the *coding* path is still 4:4:4: the
    // residual loop, the entropy contexts and CfL all step chroma over the luma extent. Refuse a
    // subsampled source rather than emit a stream that claims 4:4:4 and codes something else.
    // Lifted by the 4:2:0 (#390) and 4:2:2 (#391) slices.
    if planes.subsampling() != ChromaSubsampling::Cs444 {
        return Err(Error::unsupported(
            env!("CARGO_PKG_NAME"),
            "AV1: only 4:4:4 planes are encoded today",
        ));
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
        color_primaries: colour.primaries.code_point(),
        transfer_characteristics: colour.transfer.code_point(),
        matrix_coefficients: colour.matrix.code_point(),
        full_range: matches!(colour.range, ColorRange::Full),
    };
    // Under the §5.5.2 sRGB shortcut `color_range` is *inferred* as full and no bit is coded, so a
    // studio-range request there could not be signalled — the stream would silently claim full
    // range. Reject it rather than emit a header that disagrees with the samples.
    if config.is_srgb_shortcut() && !config.full_range {
        return Err(Error::invalid_input(
            env!("CARGO_PKG_NAME"),
            "AV1: the sRGB color_config shortcut infers full range; studio range needs a non-identity matrix",
        ));
    }

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
        // Per-plane source stride. The downscaled/upscaled widths stay luma-derived: §7.16 scales
        // them per plane by `Round2(w, subX)`, which only matters once a subsampled source can
        // reach here — `encode_with` rejects one today, and superres x subsampling is its own
        // slice.
        let mut up: [Vec<u16>; 3] = std::array::from_fn(|i| {
            crate::filter::superres_upscale_plane(
                &recon.planes[i],
                recon.geom[i].coded_w,
                coded_w as usize,
                uw,
                uh,
            )
        });
        let deblock_up = crate::filter::superres_upscale_plane(
            &recon.deblocked_luma,
            recon.geom[0].coded_w,
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
            recon.geom[0].coded_w,
            uw,
            uh,
            crate::filter::WIENER_DEFAULT,
            crate::filter::WIENER_DEFAULT,
        );
        // Each plane crops from its own coded stride to its own visible extent.
        std::array::from_fn(|i| {
            let g = recon.geom[i];
            crop(&planes[i], g.w as u32, g.coded_w as u32, g.h as u32)
        })
    };

    let still = EncodedStill {
        obus: headers::assemble_temporal_unit(&seq_payload, &frame_payload),
        config,
    };
    let recon = ReconImage {
        width,
        height,
        bit_depth: BitDepth::from_bits(recon.bit_depth).ok_or_else(|| {
            Error::unsupported(
                env!("CARGO_PKG_NAME"),
                "AV1: unsupported reconstruction bit depth",
            )
        })?,
        subsampling: planes.subsampling(),
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
    use gamut_color::Planar8;

    use super::*;

    #[test]
    fn subsampled_planes_are_rejected_until_the_chroma_coding_path_lands() {
        // The plane geometry is per-plane, but the coding path is not: the residual loop, the
        // entropy contexts and CfL all still step chroma over the luma extent. Encoding a 4:2:0
        // buffer would therefore emit a stream that claims 4:4:4 and codes something else.
        //
        // Asserted on the diagnostic rather than `is_err()`: `encode_with` has several other
        // rejections, so only the message distinguishes this guard from them.
        let planes = Planar8::from_planes_subsampled(
            8,
            8,
            gamut_color::ChromaSubsampling::Cs420,
            [vec![0; 64], vec![0; 16], vec![0; 16]],
        )
        .expect("valid 4:2:0 planes");
        let err = encode_still_intra(&planes, 40).expect_err("4:2:0 is not encodable yet");
        assert_eq!(
            err.static_message(),
            Some("AV1: only 4:4:4 planes are encoded today")
        );
        // The same buffer at 4:4:4 encodes, so the guard is keyed on the subsampling and not on
        // some other property of this input.
        let full = Planar8::from_planes(8, 8, [vec![0; 64], vec![0; 64], vec![0; 64]]).unwrap();
        assert!(encode_still_intra(&full, 40).is_ok());
    }

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
    fn encoded_bitstream_is_stable() {
        // The encoder's structural choices — block partition (should_split/decide_rect), palette
        // (decide_palette/palette_cache/signal_palette_colors), per-superblock delta-q/lf
        // (signal_delta_q/lf), skip (block_is_skippable), tx selection (select_tx_*), loop-restoration
        // signalling (write_lr) — change the OBU bitstream but reconstruct bit-exactly, so the dav1d
        // recon oracle in recon_dav1d.rs cannot see them. This snapshot pins an FNV-1a checksum of the
        // OBU stream for a spread of content patterns, sizes and quantizers; any change to a
        // (deterministic) encoder decision moves a checksum. The recon oracle proves these same streams
        // decode correctly, so the pair certifies "deterministic AND correct", not just "deterministic".
        fn fnv1a(bytes: &[u8]) -> u64 {
            bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
        }
        // Patterns exercise distinct decisions: gradient (detailed residual + range split), flat (every
        // block skippable), palette (few luma colours + flat chroma), checker (max residual / golomb
        // tails), bands (vertical structure → rectangular partitions).
        let pat = |id: u8, x: u32, y: u32| -> [u8; 3] {
            match id {
                0 => [(x * 7 + y) as u8, (x ^ (y * 3)) as u8, (x + y * 5) as u8],
                1 => [200, 100, 50],
                2 => [(((x / 4 + y / 4) % 5) * 50) as u8, 128, 128],
                3 => {
                    let v = if (x + y).is_multiple_of(2) { 0 } else { 255 };
                    [v, 255 - v, v]
                }
                _ => [((x / 8) * 40) as u8, 128, 128],
            }
        };
        let cases: &[(u8, u32, u32, u8, u64)] = &[
            (0, 40, 24, 0, 0x5728_aabc_5720_858a),
            (1, 64, 64, 1, 0xa6ac_d7ee_70b8_7653),
            (2, 32, 32, 40, 0xad7f_19dc_a8cb_bf32),
            (3, 48, 48, 16, 0x4d6f_bb67_2399_a35e),
            (4, 64, 48, 90, 0x5fed_6011_e0a0_278c),
            (0, 100, 80, 8, 0x8302_4eaa_7b86_ddc7),
            (0, 130, 70, 120, 0x3324_a4a0_9acf_6e8f),
            (2, 64, 64, 200, 0x307c_fb34_8720_4baf),
            (3, 96, 96, 60, 0x0540_b9da_a870_9e1a),
            (4, 128, 64, 30, 0x800d_d57f_9bd3_5fae),
        ];
        for &(id, w, h, q, want) in cases {
            let p = planes(w, h, |x, y| pat(id, x, y));
            let obus = encode_still_intra(&p, q).unwrap().0.obus;
            assert_eq!(
                fnv1a(&obus),
                want,
                "bitstream changed for pat{id} {w}x{h} q{q}"
            );
        }
    }

    #[test]
    fn cdf_adaptation_shrinks_the_coded_stream() {
        // `disable_cdf_update = 0` is a pure coding win: the reconstruction is unchanged (recon.rs
        // proves that against libaom and dav1d), only the symbol stream gets shorter. The bounds
        // below are the byte counts this encoder produced with *static* CDFs, measured on the
        // parent commit. Each case must now come in strictly under its bound, so silently losing
        // adaptation — the checksums in `encoded_bitstream_is_stable` would move, but so would any
        // other encoder change — is caught for what it is: a size regression.
        //
        // The margin is large (~20–35%) because a still image gives every context a whole frame to
        // converge, and the §9.4 defaults are tuned for video.
        let gradient = planes(64, 64, |x, y| {
            [(x * 3) as u8, (y * 3) as u8, ((x + y) * 2) as u8]
        });
        let noise = planes(128, 96, |x, y| {
            let v = (x
                .wrapping_mul(2_654_435_761)
                .wrapping_add(y.wrapping_mul(40503))
                >> 13) as u8;
            [v, v.wrapping_mul(3), v.wrapping_add(77)]
        });
        let smooth = planes(160, 120, |x, y| {
            let a = ((x as f32 / 9.0).sin() * 60.0 + (y as f32 / 7.0).cos() * 50.0 + 128.0) as u8;
            [a, a.wrapping_add(20), a.wrapping_sub(30)]
        });
        for (name, p, q, static_bytes) in [
            ("gradient", &gradient, 60u8, 495usize),
            ("noise", &noise, 120, 17_736),
            ("smooth", &smooth, 90, 8173),
        ] {
            let n = encode_still_intra(p, q).unwrap().0.obus.len();
            assert!(
                n < static_bytes,
                "{name} q{q}: {n} bytes, not below the static-CDF baseline of {static_bytes}"
            );
        }
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
                Err(error) if error.kind() == gamut_core::ErrorKind::InvalidInput => {
                    assert_eq!(
                        error.static_message(),
                        Some("image has a zero dimension"),
                        "{w}x{h}"
                    );
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
    fn default_colour_entry_point_matches_the_identity_one() {
        // `encode_still_intra_with(_, _, Av1Colour::default())` must be the *same* encode as
        // `encode_still_intra` — otherwise the default path's pinned checksums would not cover it.
        let p = planes(40, 24, |x, y| [(x * 3) as u8, (y * 5) as u8, (x + y) as u8]);
        for &q in &[0u8, 40] {
            let plain = encode_still_intra(&p, q).unwrap().0;
            let with = encode_still_intra_with(&p, q, Av1Colour::default())
                .unwrap()
                .0;
            assert_eq!(plain.obus, with.obus, "q{q}");
            assert_eq!(
                plain.config.matrix_coefficients,
                with.config.matrix_coefficients
            );
        }
    }

    #[test]
    fn colour_is_mirrored_into_the_config_and_the_sequence_header() {
        use gamut_color::cicp::{ColourPrimaries, MatrixCoefficients, TransferCharacteristics};

        let p = planes(40, 24, |x, y| [(x * 3) as u8, (y * 5) as u8, (x + y) as u8]);
        let colour = Av1Colour {
            primaries: ColourPrimaries::Bt2020,
            transfer: TransferCharacteristics::Srgb,
            matrix: MatrixCoefficients::Bt2020Ncl,
            range: ColorRange::Limited,
        };
        let (still, _) = encode_still_intra_with(&p, 40, colour).unwrap();
        // `gamut-avif` mirrors these straight into `av1C`/`colr`, so each field must survive.
        assert_eq!(still.config.color_primaries, 9);
        assert_eq!(still.config.transfer_characteristics, 13);
        assert_eq!(still.config.matrix_coefficients, 9);
        assert!(!still.config.full_range);
        // Still profile 1 / 4:4:4 — this change is the colour transform, not the plane geometry.
        assert_eq!(still.config.seq_profile, 1);
        assert_eq!(
            (
                still.config.chroma_subsampling_x,
                still.config.chroma_subsampling_y
            ),
            (0, 0)
        );
        // The CICP code points really reach the bitstream.
        let identity = encode_still_intra(&p, 40).unwrap().0;
        assert_ne!(
            seq_header_payload(&still.obus),
            seq_header_payload(&identity.obus)
        );
    }

    #[test]
    fn colour_range_bit_is_coded_only_outside_the_srgb_shortcut() {
        use gamut_color::cicp::{ColourPrimaries, MatrixCoefficients, TransferCharacteristics};

        // AV1 §5.5.2: with BT.709 primaries + sRGB transfer + identity matrix, `color_range` is
        // inferred and *no* bit is coded; any other combination codes it explicitly. Flipping only
        // the range must therefore change the payload for a real matrix…
        let p = planes(40, 24, |x, y| [(x * 3) as u8, (y * 5) as u8, (x + y) as u8]);
        let base = Av1Colour {
            primaries: ColourPrimaries::Bt709,
            transfer: TransferCharacteristics::Srgb,
            matrix: MatrixCoefficients::Bt709,
            range: ColorRange::Full,
        };
        let full = encode_still_intra_with(&p, 40, base).unwrap().0;
        let limited = encode_still_intra_with(
            &p,
            40,
            Av1Colour {
                range: ColorRange::Limited,
                ..base
            },
        )
        .unwrap()
        .0;
        assert_ne!(
            seq_header_payload(&full.obus),
            seq_header_payload(&limited.obus),
            "color_range must be coded when the sRGB shortcut does not apply"
        );

        // …and under the shortcut a studio-range request is refused rather than silently signalled
        // as full range.
        let shortcut = Av1Colour {
            range: ColorRange::Limited,
            ..Av1Colour::default()
        };
        match encode_still_intra_with(&p, 40, shortcut) {
            Err(error) if error.kind() == gamut_core::ErrorKind::InvalidInput => {}
            other => panic!("expected an InvalidInput for shortcut + studio range, got {other:?}"),
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
