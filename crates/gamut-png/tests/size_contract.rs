//! The size contract (issue #224): gamut's output measured against libpng at zlib level 9, with a
//! per-case budget that each carries its own written justification.
//!
//! `README.md` and `STATUS.md` have long claimed "output size is benchmarked against libpng at
//! maximum compression". `benches/encode.rs` now prints that comparison, but a bench asserts
//! nothing and is not in the per-PR gate. This file is what makes the claim enforceable: a
//! regression in the crate's reason to exist fails the build, which is the same mechanism
//! `gamut-deflate`'s ratio contract and `gamut-webp/tests/effort.rs` use.
//!
//! Budgets are *measured*, not aspirational, and they are one-sided. The table below records what
//! each case actually achieves alongside what is asserted, so drift shows up in review rather
//! than as a surprise red build. Deliberately no "budgets are still tight" assertion: it would
//! fail on a libpng point release for no correctness reason.

mod common;

use gamut_core::{Dimensions, EncodeImage, ImageRef, Rgb8, Rgba8};
use gamut_png::{FilterStrategy, Level, PngEncoder, deconstruct};

/// One case's size budget against libpng at zlib level 9.
struct Budget {
    /// Corpus entry name; matches `benches/encode.rs`.
    name: &'static str,
    /// The most gamut's file may measure as a fraction of libpng's. `1.00` reads "never larger".
    max_ratio: f64,
    /// What the case measured when the budget was set, so drift is visible in review.
    measured: f64,
    /// Why this number and not a tighter one — which stage spends the bytes.
    why: &'static str,
}

/// Every budget carries its justification. Measured at 128x128 (half the bench's side, so the
/// suite stays quick enough for the coverage and mutation lanes); the ratios track the bench's
/// 256x256 figures closely but are not identical, which is why they are recorded separately.
const BUDGETS: &[Budget] = &[
    Budget {
        name: "gradient_rgb8",
        max_ratio: 0.98,
        measured: 0.939,
        why: "no reduction applies, so this is filtering plus DEFLATE against libpng's own \
              adaptive filtering. The margin is thin by nature -- both encoders are doing the \
              same job -- so the budget only guards against losing outright.",
    },
    Budget {
        name: "photo_rgb8",
        max_ratio: 0.85,
        measured: 0.752,
        why: "smooth photographic content: palette-hostile, so again pure filtering + DEFLATE, \
              and the win is the optimal parse. Coupled to gamut-deflate's own Best/z9 column by \
              construction: if that regresses, this row moves with it. Headroom is wider than \
              the others for that reason.",
    },
    Budget {
        name: "noise_rgb8",
        max_ratio: 1.01,
        measured: 0.998,
        why: "incompressible, so both encoders fall back to stored blocks and the file is \
              slightly larger than the raw samples. Above 1.0 because there is nothing to win \
              here, not because we lose; the margin covers stored-block framing only.",
    },
    Budget {
        name: "grey_as_rgb8",
        max_ratio: 0.70,
        measured: 0.582,
        why: "R=G=B everywhere, so auto-reduce drops two channels before DEFLATE runs. A \
              structural win libpng does not attempt.",
    },
    Budget {
        name: "flat_rgba8",
        max_ratio: 0.45,
        measured: 0.321,
        why: "one opaque colour: the reduce cascade collapses it to depth-1 indexed, and chunk \
              framing is most of what remains.",
    },
    Budget {
        name: "sprite_rgba8",
        max_ratio: 1.00,
        measured: 0.963,
        why: "binary alpha over invisible colour noise. Deliberately loose: the reduce cascade \
              does not reach this case today -- no tRNS colour key, no dirty-alpha cleaning -- so \
              the margin is thin. Tightening it is the acceptance test for those two axes.",
    },
    Budget {
        name: "palette64_rgba8",
        max_ratio: 1.15,
        measured: 1.114,
        // The one row where gamut is *larger* than libpng, and the budget says so rather than
        // hiding it. A real defect the measurement found, filed separately.
        why: "gamut auto-palettises (64 colours over two alpha levels); libpng-9 writes full \
              RGBA. At 256x256 that wins by 35%, but at 128x128 it LOSES by 11%. Not because \
              `reduce::analyze8` ignores the palette chunks -- it does count them -- but because \
              it compares *raw* sizes, and raw size does not predict compressed size when one \
              candidate's bytes are incompressible and the other's are not. Measured: PLTE + \
              tRNS is a flat 273 bytes that DEFLATE cannot touch, while the indexed pixel data \
              compresses to 121 and the RGBA alternative libpng writes compresses to 405 total. \
              The estimate sees 16 664 against 65 536 and picks palette by 4x; the crossover is \
              near 160x160. The budget records the loss; a cost model that weighs incompressible \
              overhead against compressible pixels is what tightens it.",
    },
];

/// Half the bench's side, so this file stays fast enough for the coverage and mutation lanes.
const SIDE: u32 = 128;

/// The pixels for a budget row, and how many channels they carry.
fn pixels(name: &str) -> (Vec<u8>, usize) {
    let side = SIDE;
    match name {
        "gradient_rgb8" => (common::corpus::gradient_rgb(side), 3),
        "photo_rgb8" => (common::corpus::photo_rgb(side), 3),
        "noise_rgb8" => (common::corpus::noise_rgb(side), 3),
        "grey_as_rgb8" => (common::corpus::grey_as_rgb(side), 3),
        "palette64_rgba8" => (common::corpus::palette64_rgba(side), 4),
        "sprite_rgba8" => (common::corpus::sprite_rgba(side), 4),
        "flat_rgba8" => (common::corpus::flat_rgba(side), 4),
        other => panic!("unknown budget row {other}"),
    }
}

/// Encodes at the crate's smallest-output settings.
fn gamut_best(samples: &[u8], channels: usize) -> Vec<u8> {
    let encoder = PngEncoder::new()
        .with_compression(Level::Best)
        .with_filter(FilterStrategy::BruteForce)
        .with_auto_reduce(true);
    let dims = Dimensions::new(SIDE, SIDE).expect("valid dimensions");
    let mut out = Vec::new();
    if channels == 3 {
        let image = ImageRef::<Rgb8>::new(samples, dims).expect("buffer matches dimensions");
        encoder.encode_image(image, &mut out).expect("encode");
    } else {
        let image = ImageRef::<Rgba8>::new(samples, dims).expect("buffer matches dimensions");
        encoder.encode_image(image, &mut out).expect("encode");
    }
    out
}

/// The same source layout through libpng at zlib level 9 — no palette hint, default adaptive
/// filtering. Handing libpng a palette would hand it gamut's own reduction.
fn libpng9(samples: &[u8], channels: usize) -> Vec<u8> {
    let color_type = if channels == 3 {
        libpng_oracle::COLOR_RGB
    } else {
        libpng_oracle::COLOR_RGBA
    };
    libpng_oracle::encode(
        samples,
        SIDE,
        SIDE,
        color_type,
        8,
        &libpng_oracle::EncodeOpts {
            compression_level: Some(9),
            ..libpng_oracle::EncodeOpts::default()
        },
    )
}

#[test]
fn gamut_never_exceeds_its_size_budget_against_libpng9() {
    for budget in BUDGETS {
        let (samples, channels) = pixels(budget.name);
        let ours = gamut_best(&samples, channels);
        let theirs = libpng9(&samples, channels);
        let ratio = ours.len() as f64 / theirs.len() as f64;
        assert!(
            ratio <= budget.max_ratio,
            "{}: {} bytes vs libpng-9's {} = {ratio:.3}, budget {:.2} (measured {:.2} when set)\n  {}",
            budget.name,
            ours.len(),
            theirs.len(),
            budget.max_ratio,
            budget.measured,
            budget.why,
        );
    }
}

#[test]
fn gamut_beats_libpng9_where_it_claims_to() {
    // "We win here" and "we do not lose too much there" are different claims, so they are
    // different tests. The winning set is listed explicitly rather than derived from
    // `max_ratio < 1.0`: a budget loosened past 1.0 during a regression would otherwise drop out
    // of this test silently, which is exactly when it should fail.
    const WINS: &[&str] = &[
        "gradient_rgb8",
        "photo_rgb8",
        "grey_as_rgb8",
        "flat_rgba8",
        "sprite_rgba8",
    ];
    for budget in BUDGETS.iter().filter(|b| WINS.contains(&b.name)) {
        let (samples, channels) = pixels(budget.name);
        let ours = gamut_best(&samples, channels);
        let theirs = libpng9(&samples, channels);
        assert!(
            ours.len() < theirs.len(),
            "{}: claims a structural win but measured {} vs {}",
            budget.name,
            ours.len(),
            theirs.len(),
        );
    }
}

#[test]
fn the_deflate_stage_accounts_for_the_residual_gap() {
    // The attribution test, and the reason `deconstruct` is a dependency of this file. Where both
    // encoders land on the same colour type and depth, the filtered stream is identical by
    // construction, so the ratio of the *compressed* streams isolates DEFLATE from filtering and
    // from the colour-type choice. Only the rows where no reduction applies can say this.
    for name in ["gradient_rgb8", "photo_rgb8"] {
        let (samples, channels) = pixels(name);
        let ours = gamut_best(&samples, channels);
        let theirs = libpng9(&samples, channels);
        let (a, b) = (
            deconstruct(&ours).expect("gamut output deconstructs"),
            deconstruct(&theirs).expect("libpng output deconstructs"),
        );

        assert_eq!(
            (a.header.color_type, a.header.bit_depth),
            (b.header.color_type, b.header.bit_depth),
            "{name}: attribution only holds when both land on the same representation",
        );
        assert_eq!(
            a.filtered_len, b.filtered_len,
            "{name}: same representation means an identical filtered stream length",
        );
        assert!(
            a.idat_compressed <= b.idat_compressed,
            "{name}: gamut's DEFLATE stage produced {} bytes against libpng-9's {}",
            a.idat_compressed,
            b.idat_compressed,
        );
    }
}

#[test]
fn encoded_size_is_deterministic() {
    // Without this the budget table is measuring noise rather than the encoder.
    for budget in BUDGETS {
        let (samples, channels) = pixels(budget.name);
        let first = gamut_best(&samples, channels);
        let second = gamut_best(&samples, channels);
        assert_eq!(first, second, "{}: encode is not reproducible", budget.name);
    }
}
