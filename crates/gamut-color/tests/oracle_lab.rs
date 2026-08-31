//! Differential conformance tests for `gamut_color::lab` against the reference CMM (Little-CMS),
//! via the dev-only [`lcms2_oracle`] crate. The lab module's ΔE metrics, XYZ↔Lab conversions,
//! and fixed-point PCS encoders were written to replicate lcms2's `cmspcs.c` exactly (module
//! docs), so these sweeps demand **exact equality** for the integer encoders and tight f64
//! bounds for the analytic functions — measured, then asserted with margin documented inline.
//!
//! No `rand` dependency: sweeps use a fixed-seed 64-bit LCG (Knuth MMIX constants) so every run
//! exercises the identical point set.

use gamut_color::lab;

/// Fixed-seed LCG over the unit interval (top 53 bits of an MMIX step), mapped to `[lo, hi)`.
struct Lcg(u64);

impl Lcg {
    fn next_in(&mut self, lo: f64, hi: f64) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let unit = (self.0 >> 11) as f64 / (1u64 << 53) as f64;
        lo + unit * (hi - lo)
    }

    fn next_lab(&mut self) -> [f64; 3] {
        [
            self.next_in(0.0, 100.0),
            self.next_in(-128.0, 128.0),
            self.next_in(-128.0, 128.0),
        ]
    }
}

/// ΔE₀₀ vs `cmsCIE2000DeltaE` over 600 seeded random pairs plus the hue-arithmetic edge pairs
/// (hue-wrap straddles of 0°/360° and 180°, one-chroma-zero, both-achromatic). Both sides are
/// straight f64 ports of the same Sharma equations, differing only in operation order, so the
/// measured disagreement is pure rounding noise: observed max ≈ 8.5e-14 on this sweep;
/// asserted at 1e-12 for slack across compilers/libm versions.
#[test]
fn ciede2000_matches_lcms2() {
    lcms2_oracle::set_quiet_log_handler();
    let mut rng = Lcg(0x1234_5678_9ABC_DEF0);
    let mut pairs: Vec<([f64; 3], [f64; 3])> =
        (0..600).map(|_| (rng.next_lab(), rng.next_lab())).collect();
    // Hue-wrap pairs: chroma vectors just above/below the 0°/360° seam (b ≈ ±ε, a > 0) and the
    // 180° seam (a < 0, b ≈ ±ε), on both sides and crossing it.
    for eps in [1e-9, 1e-3, 0.1] {
        pairs.push(([60.0, 40.0, eps], [60.0, 40.0, -eps]));
        pairs.push(([60.0, 40.0, -eps], [55.0, 35.0, eps]));
        pairs.push(([50.0, -40.0, eps], [50.0, -40.0, -eps]));
        pairs.push(([50.0, -40.0, -eps], [45.0, -35.0, eps]));
    }
    // One chroma zero (the h̄′ = h1′ + h2′ sum convention) and both achromatic.
    pairs.push(([50.0, 0.0, 0.0], [50.0, 20.0, -20.0]));
    pairs.push(([50.0, 30.0, 10.0], [70.0, 0.0, 0.0]));
    pairs.push(([50.0, 0.0, 0.0], [60.0, 0.0, 0.0]));
    pairs.push(([0.0, 0.0, 0.0], [100.0, 0.0, 0.0]));
    assert!(pairs.len() >= 500 + 16);

    let mut max = 0.0f64;
    for &(p, q) in &pairs {
        let ours = gamut_color::delta_e_2000(p, q);
        let theirs = lcms2_oracle::cie2000_delta_e(p, q, 1.0, 1.0, 1.0);
        max = max.max((ours - theirs).abs());
    }
    assert!(max <= 1e-12, "ΔE00 max divergence {max:e}");

    // Parametric weights kl/kc/kh reach lcms2's same slots.
    let weight_sets = [
        (2.0, 1.0, 1.0),
        (1.0, 2.0, 1.0),
        (1.0, 1.0, 2.0),
        (0.5, 1.5, 2.5),
    ];
    for &(p, q) in pairs.iter().take(8) {
        for (kl, kc, kh) in weight_sets {
            let ours = lab::delta_e_2000_weighted(p, q, kl, kc, kh);
            let theirs = lcms2_oracle::cie2000_delta_e(p, q, kl, kc, kh);
            assert!(
                (ours - theirs).abs() <= 1e-12,
                "weighted ΔE00({p:?}, {q:?}, {kl}/{kc}/{kh}): {ours} vs {theirs}"
            );
        }
    }
}

/// ΔE76 vs `cmsDeltaE`: identical Euclidean formulas, so agreement is at raw f64 noise
/// (observed max ≈ 1.4e-14 on this sweep; asserted at 1e-12).
#[test]
fn delta_e_76_matches_lcms2() {
    lcms2_oracle::set_quiet_log_handler();
    let mut rng = Lcg(0x0BAD_CAFE_D00D_F00D);
    for _ in 0..600 {
        let (p, q) = (rng.next_lab(), rng.next_lab());
        let ours = gamut_color::delta_e_76(p, q);
        let theirs = lcms2_oracle::delta_e_76(p, q);
        assert!(
            (ours - theirs).abs() <= 1e-12,
            "ΔE76({p:?}, {q:?}): {ours} vs {theirs}"
        );
    }
}

/// XYZ→Lab and Lab→XYZ vs `cmsXYZ2Lab`/`cmsLab2XYZ` with the white point passed **explicitly**
/// as [`lab::D50_XYZ`], so both sides normalize by bit-identical whites and only the companding
/// arithmetic differs (lcms writes `(24/116)³` and `841/108` where we write `216/24389` and
/// `κ/116` — the same rationals). Observed max ≈ 1.7e-13 (amplified by the ×500 chroma terms);
/// asserted at 1e-12.
#[test]
fn xyz_lab_matches_lcms2_with_explicit_white() {
    lcms2_oracle::set_quiet_log_handler();
    let mut rng = Lcg(0x5EED_5EED_5EED_5EED);
    const MAX_XYZ: f64 = 1.0 + 32767.0 / 32768.0;
    for _ in 0..600 {
        let xyz = [
            rng.next_in(0.0, MAX_XYZ),
            rng.next_in(0.0, MAX_XYZ),
            rng.next_in(0.0, MAX_XYZ),
        ];
        let ours = lab::xyz_to_lab(xyz, lab::D50_XYZ);
        let theirs = lcms2_oracle::xyz_to_lab(Some(lab::D50_XYZ), xyz);
        for k in 0..3 {
            assert!(
                (ours[k] - theirs[k]).abs() <= 1e-12,
                "xyz_to_lab({xyz:?})[{k}]: {} vs {}",
                ours[k],
                theirs[k]
            );
        }

        let labv = rng.next_lab();
        let ours = lab::lab_to_xyz(labv, lab::D50_XYZ);
        let theirs = lcms2_oracle::lab_to_xyz(Some(lab::D50_XYZ), labv);
        for k in 0..3 {
            assert!(
                (ours[k] - theirs[k]).abs() <= 1e-12,
                "lab_to_xyz({labv:?})[{k}]: {} vs {}",
                ours[k],
                theirs[k]
            );
        }
    }
}

/// The same conversions against lcms2's **default** (NULL) white, which is the *rounded*
/// `0.9642/1.0/0.8249` literals rather than the ICC header rationals [`lab::D50_XYZ`] uses. The
/// whites differ by ≈ 3e-6 per component; through the cube root (×1/3 relative) and the ×500/×200
/// chroma scaling that becomes ≈ 5.4e-4 max observed (in a\*/b\*) — asserted at 2e-3, with a companion check
/// that the divergence really is above the explicit-white noise floor (i.e. the bound is loose
/// for a reason, not laziness).
#[test]
fn xyz_lab_default_white_documents_the_d50_rounding_gap() {
    lcms2_oracle::set_quiet_log_handler();
    let mut rng = Lcg(0xD50D_50D5_0D50_D50D);
    let mut max = 0.0f64;
    for _ in 0..300 {
        let xyz = [
            rng.next_in(0.0, 1.2),
            rng.next_in(0.0, 1.2),
            rng.next_in(0.0, 1.2),
        ];
        let ours = lab::xyz_to_lab(xyz, lab::D50_XYZ);
        let theirs = lcms2_oracle::xyz_to_lab(None, xyz);
        for k in 0..3 {
            let d = (ours[k] - theirs[k]).abs();
            assert!(
                d <= 2e-3,
                "xyz_to_lab({xyz:?})[{k}] vs NULL white: {} vs {}",
                ours[k],
                theirs[k]
            );
            max = max.max(d);
        }
    }
    assert!(
        max > 1e-9,
        "expected a visible white-point rounding gap, got {max:e} — did lcms2 change its D50?"
    );
}

/// Whether a pre-rounding value `d` sits in lcms2's fast-floor ambiguity window. lcms2 rounds
/// with `_cmsQuickFloorWord(d + 0.5)`, whose magic-number floor quantizes to 1/65536 **before**
/// flooring — so a `d + 0.5` within 2⁻¹⁶ below an integer rounds up where gamut-color's true
/// floor stays down (an off-by-one code). `lab.rs` documents this exact caveat; random f64
/// sweeps occasionally land in the window (measured: 1 point in 6000 samples), so those points
/// are excluded and the exactness assertion is scoped to everything else. Saturating values
/// (`d + 0.5` outside `(0, 65535)`) short-circuit before flooring on both sides and are never
/// ambiguous.
fn fast_floor_ambiguous(d: f64) -> bool {
    let s = d + 0.5;
    if s <= 0.0 || s >= 65535.0 {
        return false;
    }
    s.fract() > 1.0 - 1.0 / 32768.0 // 2⁻¹⁵ margin ⊇ the 2⁻¹⁶ quantization step
}

/// PCSXYZ encoder/decoder vs `cmsFloat2XYZEncoded`/`cmsXYZEncoded2Float`: **exact** equality
/// (outside the [`fast_floor_ambiguous`] window), over a seeded sweep plus the clamp/zero edge
/// cases (`Y <= 0` zeroes all three, negatives, above-range) and the full u16 decode sweep.
#[test]
fn pcs_xyz_codec_matches_lcms2_exactly() {
    lcms2_oracle::set_quiet_log_handler();
    let mut rng = Lcg(0x0123_4567_89AB_CDEF);
    let mut cases: Vec<[f64; 3]> = (0..2000)
        .map(|_| {
            [
                rng.next_in(-0.5, 2.5),
                rng.next_in(-0.5, 2.5),
                rng.next_in(-0.5, 2.5),
            ]
        })
        .collect();
    const MAX_XYZ: f64 = 1.0 + 32767.0 / 32768.0;
    cases.extend([
        [0.0, 0.0, 0.0],
        [1.0, 1.0, 1.0],
        [MAX_XYZ, MAX_XYZ, MAX_XYZ],
        [3.0, 1.0, -0.5],
        [1.0, 0.0, 1.0],   // Y == 0 → all zero
        [1.0, -0.25, 1.0], // Y < 0 → all zero
        [-1.0, 0.5, 2.5],
    ]);
    let mut skipped = 0usize;
    for xyz in cases {
        // Replicate the encoder's clamp to test the exact value each side floors.
        let ambiguous = xyz[1] > 0.0
            && xyz
                .iter()
                .any(|&v| fast_floor_ambiguous(v.clamp(0.0, MAX_XYZ) * 32768.0));
        if ambiguous {
            skipped += 1;
            continue;
        }
        assert_eq!(
            lab::encode_pcs_xyz(xyz),
            lcms2_oracle::xyz_encode(xyz),
            "encode_pcs_xyz({xyz:?})"
        );
    }
    assert!(skipped < 5, "unexpectedly many ambiguous points: {skipped}");
    for u in 0..=u16::MAX {
        let ours = lab::decode_pcs_xyz([u; 3]);
        let theirs = lcms2_oracle::xyz_decode([u; 3]);
        assert_eq!(ours, theirs, "decode_pcs_xyz({u})");
    }
}

/// v4 and v2 16-bit PCSLAB encoders vs `cmsFloat2LabEncoded[V2]`: **exact** equality over a
/// seeded sweep plus every clamp boundary the two encodings disagree on (`L = 100` vs the v2
/// ceiling `100.390625`, `a/b = ±128/127/127.99609375`, out-of-range and negative inputs), and
/// exact decoder equality over the full u16 sweep.
#[test]
fn pcs_lab_16_codecs_match_lcms2_exactly() {
    lcms2_oracle::set_quiet_log_handler();
    let mut rng = Lcg(0xFEED_FACE_CAFE_BEEF);
    let mut cases: Vec<[f64; 3]> = (0..2000)
        .map(|_| {
            [
                rng.next_in(-10.0, 110.0),
                rng.next_in(-140.0, 140.0),
                rng.next_in(-140.0, 140.0),
            ]
        })
        .collect();
    let edge_l = [
        0.0, 8.0, 50.0, 99.999, 100.0, 100.390624, 100.390625, 100.390626, 101.0, -1.0,
    ];
    let edge_ab = [
        -200.0,
        -128.0,
        -127.999,
        -1.0,
        0.0,
        126.999,
        127.0,
        127.001,
        127.99609375,
        127.996094,
        128.0,
        200.0,
    ];
    for &l in &edge_l {
        for &a in &edge_ab {
            for &b in &edge_ab {
                cases.push([l, a, b]);
            }
        }
    }
    let mut skipped = 0usize;
    for labv in cases {
        let [l, a, b] = labv;
        // Replicate each encoder's clamp+scale so the ambiguity window is tested on the exact
        // value both sides floor (see `fast_floor_ambiguous`).
        let v4_ambiguous = fast_floor_ambiguous(l.clamp(0.0, 100.0) * 655.35)
            || fast_floor_ambiguous((a.clamp(-128.0, 127.0) + 128.0) * 257.0)
            || fast_floor_ambiguous((b.clamp(-128.0, 127.0) + 128.0) * 257.0);
        if v4_ambiguous {
            skipped += 1;
        } else {
            assert_eq!(
                lab::encode_lab_v4_16(labv),
                lcms2_oracle::lab_encode_v4(labv),
                "encode_lab_v4_16({labv:?})"
            );
        }
        let ab_max = 65535.0 / 256.0 - 128.0;
        let v2_ambiguous = fast_floor_ambiguous(l.clamp(0.0, 100.390625) * 652.8)
            || fast_floor_ambiguous((a.clamp(-128.0, ab_max) + 128.0) * 256.0)
            || fast_floor_ambiguous((b.clamp(-128.0, ab_max) + 128.0) * 256.0);
        if v2_ambiguous {
            skipped += 1;
        } else {
            assert_eq!(
                lab::encode_lab_v2_16(labv),
                lcms2_oracle::lab_encode_v2(labv),
                "encode_lab_v2_16({labv:?})"
            );
        }
    }
    assert!(
        skipped < 10,
        "unexpectedly many ambiguous points: {skipped}"
    );
    for u in 0..=u16::MAX {
        assert_eq!(
            lab::decode_lab_v4_16([u; 3]),
            lcms2_oracle::lab_decode_v4([u; 3]),
            "decode_lab_v4_16({u})"
        );
        assert_eq!(
            lab::decode_lab_v2_16([u; 3]),
            lcms2_oracle::lab_decode_v2([u; 3]),
            "decode_lab_v2_16({u})"
        );
    }
    // Deliberately no lab_8 differential: lcms2 has no float→8-bit Lab codec (its formatters
    // widen 8-bit samples to 16-bit v2 by byte duplication); encode_lab_8 is spec-direct.
}
