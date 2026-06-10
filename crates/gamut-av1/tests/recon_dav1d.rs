//! Bit-exact reconstruction cross-check for the lossy intra path (P6 keystone).
//!
//! The encoder maintains a reconstruction buffer that must equal, sample for sample, what a
//! conformant decoder produces. This test encodes a lossy still, decodes the raw AV1 OBU stream
//! (a Section-5 low-overhead stream — each OBU carries its size) with the real `dav1d` decoder, and
//! asserts the decoded planes equal the encoder's exported reconstruction byte-for-byte.
//!
//! `dav1d` is linked in from the `third_party/dav1d` submodule via the `dav1d-oracle`
//! dev-dependency, so the check is hermetic and always runs — it never depends on a `dav1d` binary
//! being installed on the host. Building these tests therefore needs meson/ninja/nasm and the
//! checked-out submodule (`git submodule update --init --recursive`).

use gamut_av1::encode_still_intra;
use gamut_color::Planar8;

/// Builds identity planes (Y=G, U=B, V=R) from an RGB generator.
fn planes(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) -> Planar8 {
    let mut rgb = vec![0u8; (w * h * 3) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 3) as usize;
            rgb[i..i + 3].copy_from_slice(&f(x, y));
        }
    }
    Planar8::from_rgb8_identity(&rgb, w, h).unwrap()
}

/// Encodes `planes` at `qindex`, decodes the OBU stream with dav1d, and asserts the decoded planes
/// equal the encoder's reconstruction.
fn check(planes: &Planar8, qindex: u8) {
    let (still, recon) = encode_still_intra(planes, qindex).unwrap();
    let (w, h) = (recon.width as usize, recon.height as usize);

    // A standalone Section-5 OBU stream for dav1d needs a temporal-delimiter OBU first (AVIF omits
    // it inside the container). TD = obu_type 2, has_size_field, empty payload.
    let mut stream = vec![0x12u8, 0x00];
    stream.extend_from_slice(&still.obus);

    let decoded = dav1d_oracle::decode_obu(&stream)
        .unwrap_or_else(|e| panic!("dav1d decode failed for {w}x{h} q{qindex}: {e}"));
    assert_eq!(
        (decoded.width as usize, decoded.height as usize),
        (w, h),
        "decoded dimensions differ from reconstruction for {w}x{h} q{qindex}"
    );

    for (p, (dec, enc)) in decoded.planes.iter().zip(&recon.planes).enumerate() {
        assert_eq!(
            dec, enc,
            "plane {p} mismatch (decoder vs encoder reconstruction) for {w}x{h} q{qindex}"
        );
    }
}

#[test]
fn lossy_reconstruction_matches_dav1d() {
    // A photographic-ish gradient with texture: exercises non-trivial residuals, DC prediction
    // across block boundaries, and the all-zero (txb_skip) path on flat regions.
    let texture = |x: u32, y: u32| {
        let r = (x.wrapping_mul(3).wrapping_add(y) % 256) as u8;
        let g = ((x + y.wrapping_mul(2)) % 256) as u8;
        let b = (128 + ((x ^ y) % 64)) as u8;
        [r, g, b]
    };
    // qindex stays in 1..=20 so the coefficient-CDF quantizer context is 0 (matches the static CDFs).
    for &q in &[4u8, 12, 20] {
        for &(w, h) in &[(8, 8), (17, 13), (32, 32), (64, 48), (100, 70)] {
            check(&planes(w, h, texture), q);
        }
    }
}

#[test]
fn lossy_reconstruction_matches_dav1d_all_qctx() {
    // The same textured content, but at quantizers spanning every coefficient-CDF quantizer
    // context: qctx 1 (21..=60), qctx 2 (61..=120) and qctx 3 (121..=255). A wrong CDF table makes
    // the arithmetic decode diverge, so dav1d byte-equality is a hard correctness gate per qctx.
    let texture = |x: u32, y: u32| {
        let r = (x.wrapping_mul(5).wrapping_add(y.wrapping_mul(3)) % 256) as u8;
        let g = ((x.wrapping_add(y).wrapping_mul(2)) % 256) as u8;
        let b = (64 + ((x.wrapping_mul(7) ^ y) % 128)) as u8;
        [r, g, b]
    };
    // One representative qindex per context boundary, plus the extremes.
    for &q in &[21u8, 40, 60, 61, 90, 120, 121, 200, 255] {
        for &(w, h) in &[(8, 8), (17, 13), (40, 24), (100, 70)] {
            check(&planes(w, h, texture), q);
        }
    }
}

#[test]
fn tx_type_selection_matches_dav1d() {
    // Content engineered so the encoder's per-block transform-type search picks non-DCT_DCT types
    // from TX_SET_INTRA_2 (IDTX on sharp screen-content edges, ADST on directional ramps). dav1d
    // must decode whatever type was signaled to the encoder's reconstruction byte-for-byte, so this
    // exercises the ADST/IDTX inverse transforms end-to-end through the real decoder.
    let screen = |x: u32, y: u32| {
        // 1-pixel-wide vertical bars + a diagonal ramp: high-frequency, impulse-like residuals.
        let bar = if x.is_multiple_of(2) { 235 } else { 20 } as u8;
        let ramp = ((x.wrapping_add(y)).wrapping_mul(9) % 256) as u8;
        let diag = if (x + y).is_multiple_of(7) { 250 } else { 40 } as u8;
        [bar, ramp, diag]
    };
    for &q in &[10u8, 32, 80, 160] {
        for &(w, h) in &[(8, 8), (16, 16), (37, 21), (64, 40)] {
            check(&planes(w, h, screen), q);
        }
    }
}

#[test]
fn directional_modes_match_dav1d() {
    // Strong vertical, horizontal and diagonal structure so the mode search picks the directional
    // modes (V/H/D135/D113/D157). dav1d must decode each signaled angle to the encoder
    // reconstruction byte-for-byte, exercising the directional prediction process end-to-end.
    let directional = |x: u32, y: u32| {
        let vert = if (x / 2).is_multiple_of(2) { 210 } else { 30 } as u8; // vertical bars
        let horiz = if (y / 2).is_multiple_of(2) { 200 } else { 40 } as u8; // horizontal bars
        let diag = (((x + y) * 16) % 256) as u8; // 45° ramp
        [vert, horiz, diag]
    };
    for &q in &[8u8, 28, 96, 180] {
        for &(w, h) in &[(8, 8), (16, 16), (37, 21), (64, 40)] {
            check(&planes(w, h, directional), q);
        }
    }
}

#[test]
fn filter_intra_modes_match_dav1d() {
    // Mixed flat-plus-fine-texture content: the encoder's per-block search picks recursive
    // filter-intra (§7.11.2.3, signaled as a DC_PRED block + `use_filter_intra` + a
    // `filter_intra_mode`) on many blocks. dav1d must run the same recursive predictor and reach the
    // encoder's reconstruction byte-for-byte. (The encoder selects all five filter modes on this
    // content, so it covers FILTER_DC/V/H/D157/PAETH through the real decoder.)
    let textured = |x: u32, y: u32| {
        let r = (x.wrapping_mul(3).wrapping_add(y) % 256) as u8;
        let g = ((x + y.wrapping_mul(2)) % 256) as u8;
        let b = (128 + ((x ^ y) % 64)) as u8;
        [r, g, b]
    };
    for &q in &[6u8, 24, 88, 170] {
        for &(w, h) in &[(8, 8), (16, 16), (37, 21), (64, 40)] {
            check(&planes(w, h, textured), q);
        }
    }
}

#[test]
fn cfl_chroma_from_luma_matches_dav1d() {
    // Chroma that tracks the luma high-frequency: U falls as luma rises (negative alpha), V rises
    // with it (positive alpha). The encoder's per-block CfL search then signals uv_mode = UV_CFL_PRED
    // with non-zero CflAlphaU/CflAlphaV, and dav1d must run §7.11.5 chroma-from-luma to the encoder's
    // reconstruction byte-for-byte — exercising both alpha signs and read_cfl_alphas end-to-end.
    let cfl = |x: u32, y: u32| {
        let base = ((x.wrapping_mul(7).wrapping_add(y.wrapping_mul(5))) % 200) as i32 + 28; // luma
        let g = base as u8;
        let r = (base / 2 + 100).clamp(0, 255) as u8; // tracks +luma
        let b = (220 - base / 2).clamp(0, 255) as u8; // tracks -luma
        [r, g, b]
    };
    for &q in &[6u8, 24, 88, 170] {
        for &(w, h) in &[(8, 8), (16, 16), (37, 21), (64, 40)] {
            check(&planes(w, h, cfl), q);
        }
    }
}

#[test]
fn deblock_matches_dav1d() {
    // Block-aligned flat tiles with moderate steps between them: after quantization the 4×4 block
    // boundaries carry exactly the small discontinuities the deblocking loop filter (§7.14) smooths.
    // The encoder applies the filter to its reconstruction and dav1d applies it on decode, so the
    // byte-for-byte match validates the narrow-filter math, masks, and the vertical-then-horizontal
    // pass ordering across the full quantizer (and hence loop-filter-level) range.
    let tiles = |x: u32, y: u32| {
        let step = ((x / 4 + y / 4) % 6) as u8; // changes every 4 px ⇒ on the block grid
        let v = 60u8.wrapping_add(step.wrapping_mul(18));
        [
            v,
            v.wrapping_add(20),
            200u8.wrapping_sub(step.wrapping_mul(12)),
        ]
    };
    for &q in &[16u8, 48, 110, 200] {
        for &(w, h) in &[(8, 8), (16, 16), (35, 23), (64, 40)] {
            check(&planes(w, h, tiles), q);
        }
    }
}

#[test]
fn mixed_4x4_and_8x8_blocks_match_dav1d() {
    // Content with both smooth regions (low local range ⇒ the encoder codes a single 8×8 block with
    // TX_8X8) and high-frequency regions (⇒ split to 4×4). An 8×8 transform spans two MI cells, so its
    // coefficient contexts that accumulate over neighbours — `dc_sign`, `txb_skip`, the level context
    // — must sum across both cells. When an 8×8 block borders 4×4 blocks with non-uniform DC signs,
    // reading a single cell diverges from a conformant decoder, so this is the gate for that mix.
    let mixed = |x: u32, y: u32| {
        // 16×16 smooth tiles (each a flat-ish gradient) separated by sharp seams every 16 px.
        let smooth = (((x % 16) + (y % 16)) * 2) as i32 + 40;
        let seam = if x.is_multiple_of(16) || y.is_multiple_of(16) {
            200
        } else {
            0
        };
        let v = (smooth + seam).clamp(0, 255);
        let r = (v / 2 + 90).clamp(0, 255); // chroma tracks luma (drives CfL on 8×8 blocks)
        let b = (210 - v / 2).clamp(0, 255);
        [r as u8, v as u8, b as u8]
    };
    for &q in &[21u8, 64, 130, 220] {
        // Sizes that force the partial-superblock / padding paths around the 8×8 blocks.
        for &(w, h) in &[(40, 40), (100, 70), (90, 96)] {
            check(&planes(w, h, mixed), q);
        }
    }
}

#[test]
fn cdef_matches_dav1d() {
    // Strong directional structure (diagonal/edged content) gives the CDEF direction search (§7.15.2)
    // distinct per-8×8 directions and non-trivial variance, so the primary+secondary deringing filter
    // (§7.15.3) actually fires. The encoder runs deblock → CDEF on its reconstruction and dav1d does
    // the same on decode, so byte-equality validates the direction search, constrain, taps, and the
    // out-of-frame sample availability — at quantizers spanning the signaled CDEF strength range.
    let edged = |x: u32, y: u32| {
        let d = ((x + y) % 16) as u8; // 45° structure on the 8×8 CDEF grid
        let r = if d < 8 { 40 } else { 210 };
        let g = (30u8).wrapping_add(d.wrapping_mul(13));
        let b = if (x.wrapping_sub(y)) % 12 < 6 {
            70
        } else {
            190
        };
        [r, g, b]
    };
    for &q in &[32u8, 80, 128, 220] {
        for &(w, h) in &[(8, 8), (16, 16), (35, 23), (64, 40)] {
            check(&planes(w, h, edged), q);
        }
    }
}

#[test]
fn flat_lossy_reconstruction_matches_dav1d() {
    // A solid color: every residual quantizes to zero, so the reconstruction is the DC prediction
    // chain — a clean test that prediction-from-reconstruction tracks the decoder exactly.
    check(&planes(48, 40, |_, _| [200, 100, 50]), 16);
}

#[test]
fn directional_and_filter_intra_8x8_match_dav1d() {
    // Smooth low-amplitude ramps keep each 8×8 block below the split threshold, so the encoder codes
    // them as single TX_8X8 blocks — and their oriented gradients drive the 8×8 directional search,
    // where MiSize ≥ BLOCK_8X8 signals `angle_delta_y` (§5.11.42) and prediction follows the general
    // directional process (§7.11.2.4). Three orientations sweep the angle space across all four zones
    // (cardinal / zone-1 / zone-2 / zone-3) and non-zero angle deltas; the residual texture also
    // selects 8×8 recursive filter-intra. Byte-equality with dav1d validates the new angle signaling,
    // the 16-sample reference extension, and the size-generic filter-intra predictor.
    let tilted_h = |x: u32, y: u32| {
        let v = (50 + (x as i32 * 4 + y as i32) / 3).clamp(0, 255);
        [
            (v / 2 + 80).clamp(0, 255) as u8,
            v as u8,
            (200 - v / 2).clamp(0, 255) as u8,
        ]
    };
    let diagonal = |x: u32, y: u32| {
        let v = (40 + (x as i32 + y as i32) / 2).clamp(0, 255);
        [
            (v / 2 + 80).clamp(0, 255) as u8,
            v as u8,
            (200 - v / 2).clamp(0, 255) as u8,
        ]
    };
    let tilted_v = |x: u32, y: u32| {
        let v = (50 + (y as i32 * 4 + x as i32) / 3).clamp(0, 255);
        [
            (v / 2 + 80).clamp(0, 255) as u8,
            v as u8,
            (200 - v / 2).clamp(0, 255) as u8,
        ]
    };
    for f in [
        &tilted_h as &dyn Fn(u32, u32) -> [u8; 3],
        &diagonal,
        &tilted_v,
    ] {
        for &q in &[21u8, 64, 130, 220] {
            for &(w, h) in &[(64u32, 64u32), (40, 40), (90, 72)] {
                check(&planes(w, h, f), q);
            }
        }
    }
}

#[test]
fn transform_16x16_blocks_match_dav1d() {
    // Very-low-amplitude ramps keep each 16×16 block under the split threshold, so the encoder codes
    // them as single TX_16X16 blocks (PARTITION_NONE at BLOCK_16X16) — exercising the 256-coefficient
    // scan/CDFs (Eob_Pt_256, the txSzCtx-2 coeff tables), the per-`intraDir` 16×16 transform-type CDF,
    // 16×16 directional/filter-intra prediction, and — between adjacent 16×16 luma blocks — the wide
    // `filterSize == 16` deblock filter (§7.14.6.4 with log2Size = 4) plus its flatMask2. The three
    // orientations sweep the angle space; byte-equality with dav1d validates every new path. Sizes
    // are ≥ a superblock fraction so 16×16 blocks form in the interior at offsets that are multiples
    // of four MI cells.
    let diagonal = |x: u32, y: u32| {
        let v = (40 + (x as i32 + y as i32) / 8).clamp(0, 255);
        [
            (v / 2 + 80).clamp(0, 255) as u8,
            v as u8,
            (200 - v / 2).clamp(0, 255) as u8,
        ]
    };
    let tilted = |x: u32, y: u32| {
        let v = (50 + (x as i32 * 2 + y as i32) / 6).clamp(0, 255);
        [
            (v / 2 + 80).clamp(0, 255) as u8,
            v as u8,
            (200 - v / 2).clamp(0, 255) as u8,
        ]
    };
    let patches = |x: u32, y: u32| {
        let v = (60 + ((x / 16 + y / 16) % 3) as i32 * 6).clamp(0, 255);
        [
            (v / 2 + 80).clamp(0, 255) as u8,
            v as u8,
            (200 - v / 2).clamp(0, 255) as u8,
        ]
    };
    for f in [&diagonal as &dyn Fn(u32, u32) -> [u8; 3], &tilted, &patches] {
        for &q in &[21u8, 64, 130, 220] {
            for &(w, h) in &[(96u32, 96u32), (64, 64), (100, 80)] {
                check(&planes(w, h, f), q);
            }
        }
    }
}

#[test]
fn transform_32x32_blocks_match_dav1d() {
    // Near-flat ramps (slope < 1/px over 32 px) keep whole 32×32 regions below the split threshold,
    // so the encoder codes them as single TX_32X32 blocks (PARTITION_NONE at BLOCK_32X32). This
    // exercises the 1024-coefficient scan/CDFs (Eob_Pt_1024 — which, unlike the smaller eob classes,
    // has no neighbour-context dimension — and the txSzCtx-3 coeff tables), the `dqDenom = 2`
    // dequantization divisor unique to 32×32 (§7.12.3), DCT_DCT-only coding (TX_SET_DCTONLY ⇒ no
    // transform-type symbol), and 32×32 DC/smooth/directional/filter-intra prediction. Adjacent 32×32
    // luma edges deblock at filterSize 16 (the cap). Byte-equality with dav1d validates every path.
    let diagonal = |x: u32, y: u32| {
        let v = (48 + (x as i32 + y as i32) / 16).clamp(0, 255);
        [
            (v / 2 + 80).clamp(0, 255) as u8,
            v as u8,
            (200 - v / 2).clamp(0, 255) as u8,
        ]
    };
    let tilted = |x: u32, y: u32| {
        let v = (40 + (x as i32 * 2 + y as i32) / 20).clamp(0, 255);
        [
            (v / 2 + 80).clamp(0, 255) as u8,
            v as u8,
            (200 - v / 2).clamp(0, 255) as u8,
        ]
    };
    let patches = |x: u32, y: u32| {
        let v = (70 + ((x / 32 + y / 32) % 3) as i32 * 5).clamp(0, 255);
        [
            (v / 2 + 80).clamp(0, 255) as u8,
            v as u8,
            (200 - v / 2).clamp(0, 255) as u8,
        ]
    };
    for f in [&diagonal as &dyn Fn(u32, u32) -> [u8; 3], &tilted, &patches] {
        for &q in &[21u8, 64, 130, 220] {
            for &(w, h) in &[(96u32, 96u32), (72, 68), (128, 96)] {
                check(&planes(w, h, f), q);
            }
        }
    }
}

#[test]
fn variable_tx_size_match_dav1d() {
    // Moderately-textured smooth blocks (low enough range to stay PARTITION_NONE, high enough that
    // the encoder splits the transform): under TX_MODE_SELECT a ≥8×8 luma block signals tx_depth and
    // uses one block-size prediction mode with several smaller square sub-transforms, while 4:4:4
    // chroma keeps one block-size transform. Exercises tx_depth 1 and 2 across 8×8/16×16/32×32 blocks,
    // the per-transform-block BlockDecoded update (directional sub-transforms see their just-coded
    // siblings), the luma txb_skip neighbour context, and the per-plane deblock (luma at the sub-tx
    // size, chroma at the block size). Byte-equality with dav1d validates all of it.
    let ramp = |x: u32, y: u32| {
        let v = (48 + ((x % 32 + y % 32) as i32) / 2).clamp(0, 255);
        [
            (v / 2 + 80).clamp(0, 255) as u8,
            v as u8,
            (200 - v / 2).clamp(0, 255) as u8,
        ]
    };
    let blocky = |x: u32, y: u32| {
        let v = (40 + ((x % 16 + y % 16) as i32)).clamp(0, 255);
        [
            (v / 2 + 80).clamp(0, 255) as u8,
            v as u8,
            (200 - v / 2).clamp(0, 255) as u8,
        ]
    };
    for f in [&ramp as &dyn Fn(u32, u32) -> [u8; 3], &blocky] {
        for &q in &[14u8, 48, 110, 200] {
            for &(w, h) in &[(64u32, 64u32), (40, 40), (96, 72)] {
                check(&planes(w, h, f), q);
            }
        }
    }
}

#[test]
fn delta_q_match_dav1d() {
    // delta_q_present: each superblock's first block signals a per-SB delta_q, so CurrentQIndex (and
    // the dc/ac dequantizer step) varies across superblocks while the coefficient-CDF qctx stays at
    // its frame value (init_coeff_cdfs derives it from base_q_idx, §8.3.2). Multi-superblock sizes
    // (> 64 px) exercise several deltas; base_q_idx values sit on the qctx boundaries (20/60/120) so
    // a ±delta pushes CurrentQIndex across the boundary without changing qctx — the case that would
    // desync if qctx tracked CurrentQIndex. Byte-equality with dav1d validates the per-block quantizer
    // tracking and the frame-level qctx.
    let texture = |x: u32, y: u32| {
        let r = (x.wrapping_mul(3).wrapping_add(y) % 256) as u8;
        let g = ((x + y.wrapping_mul(2)) % 256) as u8;
        let b = (128 + ((x ^ y) % 64)) as u8;
        [r, g, b]
    };
    for &q in &[20u8, 21, 60, 61, 120, 121, 200] {
        for &(w, h) in &[(64u32, 64u32), (96, 96), (160, 96), (100, 70)] {
            check(&planes(w, h, texture), q);
        }
    }
}

#[test]
fn skip_blocks_match_dav1d() {
    // Large flat regions: interior blocks are perfectly DC-predicted (residual identically zero), so
    // the encoder codes them with skip = 1 (no residual; reconstruction = prediction). This exercises
    // the skip flag + its neighbour context, the reset of the level/dc coefficient contexts, and the
    // CDEF rule that an all-skip 8×8 block is not filtered (§7.15.1). A solid colour makes most
    // interior blocks skip; a two-region split adds skip/non-skip neighbour-context variety.
    let solid = |_x: u32, _y: u32| [180u8, 90, 40];
    let halves = |x: u32, _y: u32| {
        if x < 48 {
            [180u8, 90, 40]
        } else {
            [60, 150, 200]
        }
    };
    for &q in &[16u8, 48, 110, 200] {
        for &(w, h) in &[(64u32, 64u32), (96, 80), (128, 96)] {
            check(&planes(w, h, solid), q);
            check(&planes(w, h, halves), q);
        }
    }
}
