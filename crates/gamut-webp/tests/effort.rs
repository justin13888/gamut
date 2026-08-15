//! The compression-effort ladder's contract (issue #261), across every rung `0..=6`.
//!
//! Three properties, in decreasing order of importance:
//!
//! 1. **Correctness is effort-independent.** A lossless encode reproduces its input bit-exactly at
//!    every rung; effort only chooses how hard the encoder searches.
//! 2. **Size is non-increasing in effort.** The VP8L ladder guarantees this *by construction* — a
//!    rung's candidate plans are a superset of the rung below's, and ties resolve to the earlier
//!    plan — so it is asserted exactly, with no tolerance.
//! 3. **Encoding is deterministic.** Two encodes at the same rung are byte-identical, which is what
//!    catches floating point or hash-iteration order leaking into an encoder decision.
//!
//! Fixtures stay small on purpose: the top rungs are the slowest code in the crate and this suite
//! runs inside the coverage and mutation-testing lanes.

use gamut_core::{DecodeImage, Dimensions, EncodeImage, ImageBuf, ImageRef, Rgb8, Rgba8};
use gamut_webp::{Effort, WebpDecoder, WebpEncoder};

/// Every rung of the ladder, lowest first.
fn all_efforts() -> Vec<Effort> {
    (0..=6)
        .map(|l| Effort::from_level(l).expect("0..=6"))
        .collect()
}

fn dims(width: u32, height: u32) -> Dimensions {
    Dimensions { width, height }
}

/// A smooth ramp — compressible, and the case the spatial transforms are for.
fn ramp_rgba(w: u32, h: u32) -> Vec<u8> {
    (0..w * h)
        .flat_map(|i| {
            let (x, y) = (i % w, i / w);
            [
                (x * 3) as u8,
                (y * 5) as u8,
                (x + y) as u8,
                0xff_u8.saturating_sub((x / 4) as u8),
            ]
        })
        .collect()
}

/// A handful of distinct colours — the palette path.
fn palette_rgba(w: u32, h: u32) -> Vec<u8> {
    const COLOURS: [[u8; 4]; 5] = [
        [0, 0, 0, 255],
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 128],
        [255, 255, 255, 255],
    ];
    (0..w * h)
        .flat_map(|i| COLOURS[(i as usize * 7 / 3) % COLOURS.len()])
        .collect()
}

/// High-entropy content: nothing should compress it much, so it exercises the paths that give up.
fn noisy_rgba(w: u32, h: u32) -> Vec<u8> {
    let mut state = 0x1234_5678u32;
    (0..w * h * 4)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect()
}

/// A single flat colour — the degenerate case where every alphabet is single-symbol.
fn flat_rgba(w: u32, h: u32) -> Vec<u8> {
    [0x20u8, 0x40, 0x60, 0xff].repeat((w * h) as usize)
}

/// The fixture corpus as `(label, pixels, dimensions)`.
fn corpus() -> Vec<(&'static str, Vec<u8>, Dimensions)> {
    vec![
        ("ramp", ramp_rgba(64, 48), dims(64, 48)),
        ("palette", palette_rgba(48, 32), dims(48, 32)),
        ("noisy", noisy_rgba(32, 24), dims(32, 24)),
        ("flat", flat_rgba(40, 24), dims(40, 24)),
        ("single-pixel", flat_rgba(1, 1), dims(1, 1)),
        ("one-row", ramp_rgba(37, 1), dims(37, 1)),
        ("one-column", ramp_rgba(1, 29), dims(1, 29)),
    ]
}

fn encode_lossless(effort: Effort, px: &[u8], d: Dimensions) -> Vec<u8> {
    let mut out = Vec::new();
    WebpEncoder::lossless()
        .with_effort(effort)
        .encode_image(ImageRef::<Rgba8>::new(px, d).expect("fixture"), &mut out)
        .expect("lossless encode");
    out
}

fn encode_lossy(effort: Effort, quality: u8, px: &[u8], d: Dimensions) -> Vec<u8> {
    let mut out = Vec::new();
    WebpEncoder::lossy(quality)
        .with_effort(effort)
        .encode_image(ImageRef::<Rgba8>::new(px, d).expect("fixture"), &mut out)
        .expect("lossy encode");
    out
}

#[test]
fn every_effort_level_round_trips_losslessly() {
    // The guarantee that must never depend on effort: lossless is bit-exact at every rung, for
    // every shape, including the degenerate 1xN / Nx1 / single-pixel cases.
    for (label, px, d) in corpus() {
        for effort in all_efforts() {
            let file = encode_lossless(effort, &px, d);
            let decoded: ImageBuf<Rgba8> =
                WebpDecoder::new().decode_image(&file).unwrap_or_else(|e| {
                    panic!("{label} at effort {}: decode failed: {e}", effort.level())
                });
            assert_eq!(
                decoded.dimensions(),
                d,
                "{label} at effort {}",
                effort.level()
            );
            assert_eq!(
                decoded.as_samples(),
                px.as_slice(),
                "{label} at effort {} is not bit-exact",
                effort.level()
            );
        }
    }
}

#[test]
fn lossless_size_is_non_increasing_in_effort() {
    // The VP8L ladder's central invariant, guaranteed by construction: each rung's candidate plans
    // extend the rung below's, and ties resolve to the earlier plan. Asserted exactly — if anyone
    // turns the ladder into a "different set per rung" table, this fails.
    for (label, px, d) in corpus() {
        let sizes: Vec<usize> = all_efforts()
            .into_iter()
            .map(|e| encode_lossless(e, &px, d).len())
            .collect();
        for level in 1..sizes.len() {
            assert!(
                sizes[level] <= sizes[level - 1],
                "{label}: effort {level} grew to {} from {} at effort {}",
                sizes[level],
                sizes[level - 1],
                level - 1
            );
        }
    }
}

#[test]
fn encoding_is_deterministic_at_every_effort() {
    // Any floating point or hash-iteration order reaching an encoder decision would show up here
    // as a byte difference between two runs of the same configuration.
    for (label, px, d) in corpus() {
        for effort in all_efforts() {
            assert_eq!(
                encode_lossless(effort, &px, d),
                encode_lossless(effort, &px, d),
                "{label}: lossless effort {} is not deterministic",
                effort.level()
            );
            assert_eq!(
                encode_lossy(effort, 70, &px, d),
                encode_lossy(effort, 70, &px, d),
                "{label}: lossy effort {} is not deterministic",
                effort.level()
            );
        }
    }
}

#[test]
fn every_effort_level_produces_a_decodable_lossy_file() {
    // Lossy output is not bit-exact, so the contract is weaker: every rung must still produce a
    // file the decoder reads back at the right shape, with the alpha plane exact (alpha is stored
    // losslessly at every rung — effort never makes it lossy).
    for (label, px, d) in corpus() {
        for effort in all_efforts() {
            let file = encode_lossy(effort, 70, &px, d);
            let decoded: ImageBuf<Rgba8> =
                WebpDecoder::new().decode_image(&file).unwrap_or_else(|e| {
                    panic!("{label} at effort {}: decode failed: {e}", effort.level())
                });
            assert_eq!(
                decoded.dimensions(),
                d,
                "{label} at effort {}",
                effort.level()
            );
            let got: Vec<u8> = decoded.as_samples().chunks_exact(4).map(|p| p[3]).collect();
            let want: Vec<u8> = px.chunks_exact(4).map(|p| p[3]).collect();
            assert_eq!(
                got,
                want,
                "{label} at effort {}: alpha must stay lossless",
                effort.level()
            );
        }
    }
}

#[test]
fn effort_does_not_disturb_the_rgb_surface() {
    // The `Rgb8` path shares the codestream encoders with `Rgba8` but not the container decisions,
    // so pin that it too round-trips at both extremes of the ladder.
    let rgb: Vec<u8> = (0..32u32 * 24)
        .flat_map(|i| [(i % 251) as u8, (i % 37) as u8, (i % 199) as u8])
        .collect();
    let d = dims(32, 24);
    for effort in [Effort::Fastest, Effort::Slowest] {
        let mut out = Vec::new();
        WebpEncoder::lossless()
            .with_effort(effort)
            .encode_image(ImageRef::<Rgb8>::new(&rgb, d).expect("fixture"), &mut out)
            .expect("encode");
        let decoded: ImageBuf<Rgb8> = WebpDecoder::new().decode_image(&out).expect("decode");
        assert_eq!(decoded.as_samples(), rgb.as_slice());
    }
}

/// Encodes losslessly with near-lossless preprocessing at `strength`.
fn encode_near_lossless(strength: u8, px: &[u8], d: Dimensions) -> Vec<u8> {
    let mut out = Vec::new();
    WebpEncoder::lossless()
        .with_near_lossless(Some(
            gamut_webp::NearLossless::new(strength).expect("0..=99"),
        ))
        .encode_image(ImageRef::<Rgba8>::new(px, d).expect("fixture"), &mut out)
        .expect("near-lossless encode");
    out
}

#[test]
fn near_lossless_off_is_byte_identical_to_plain_lossless() {
    // `None` must be a true no-op, not merely "close enough" — otherwise every existing caller
    // silently changes output the day the knob lands.
    for (label, px, d) in corpus() {
        let mut with_none = Vec::new();
        WebpEncoder::lossless()
            .with_near_lossless(None)
            .encode_image(
                ImageRef::<Rgba8>::new(&px, d).expect("fixture"),
                &mut with_none,
            )
            .expect("encode");
        assert_eq!(
            with_none,
            encode_lossless(Effort::default(), &px, d),
            "{label}: near-lossless None must be byte-identical"
        );
    }
}

#[test]
fn near_lossless_keeps_rgb_within_the_bound_and_alpha_exact() {
    // The contract callers rely on. The stream itself is still bit-exact lossless — it just codes
    // a quantized image — so what is checked is the distance from the *original*.
    for (label, px, d) in corpus() {
        for strength in [0u8, 40, 60, 99] {
            let bound = gamut_webp::NearLossless::new(strength)
                .expect("0..=99")
                .max_deviation();
            let file = encode_near_lossless(strength, &px, d);
            let decoded: ImageBuf<Rgba8> = WebpDecoder::new().decode_image(&file).expect("decode");
            assert_eq!(decoded.dimensions(), d, "{label} at strength {strength}");
            for (i, (before, after)) in px
                .chunks_exact(4)
                .zip(decoded.as_samples().chunks_exact(4))
                .enumerate()
            {
                assert_eq!(
                    before[3], after[3],
                    "{label} at strength {strength}: alpha moved at pixel {i}"
                );
                for c in 0..3 {
                    assert!(
                        u16::from(before[c].abs_diff(after[c])) <= bound,
                        "{label} at strength {strength}: channel {c} moved {} at pixel {i}, bound {bound}",
                        before[c].abs_diff(after[c])
                    );
                }
            }
        }
    }
}

#[test]
fn near_lossless_shrinks_smooth_content() {
    // A knob that never shrinks anything is not implemented, only plumbed. The fixture is a smooth
    // gradient carrying low-amplitude dither — photographic content, and the case the technique is
    // for. A pure ramp would not do: the spatial predictor already drives it to all-zero residuals,
    // so there are no low bits left to discard. Noise and flat colour are the other extremes, where
    // there is respectively nothing predictable and nothing left to remove.
    let (w, h) = (64u32, 48u32);
    let mut state = 0x9e37_79b9u32;
    let px: Vec<u8> = (0..w * h)
        .flat_map(|i| {
            let (x, y) = (i % w, i / w);
            let base = [(x * 2) as u8, (y * 2) as u8, (x + y) as u8];
            let mut dithered = [0u8; 4];
            for (c, out) in base.iter().zip(dithered.iter_mut()) {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                *out = c.wrapping_add(((state >> 28) as u8) & 0x0f);
            }
            dithered[3] = 0xff;
            dithered
        })
        .collect();
    let d = dims(w, h);
    let exact = encode_lossless(Effort::default(), &px, d).len();
    let gentle = encode_near_lossless(80, &px, d).len();
    let aggressive = encode_near_lossless(0, &px, d).len();
    assert!(
        gentle < exact,
        "gentle near-lossless ({gentle}) must beat exact ({exact})"
    );
    assert!(
        aggressive < gentle,
        "aggressive near-lossless ({aggressive}) must beat gentle ({gentle})"
    );
}

#[test]
fn near_lossless_is_ignored_by_the_lossy_path() {
    // Near-lossless is a VP8L preprocessing step. The lossy path documents that it ignores the
    // knob, so pin that rather than leaving it to the reader's assumption.
    let (w, h) = (32u32, 24u32);
    let px = ramp_rgba(w, h);
    let d = dims(w, h);
    let mut with_nl = Vec::new();
    WebpEncoder::lossy(70)
        .with_near_lossless(Some(gamut_webp::NearLossless::new(0).expect("0..=99")))
        .encode_image(
            ImageRef::<Rgba8>::new(&px, d).expect("fixture"),
            &mut with_nl,
        )
        .expect("encode");
    assert_eq!(with_nl, encode_lossy(Effort::default(), 70, &px, d));
}
