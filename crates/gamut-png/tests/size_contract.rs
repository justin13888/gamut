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
    /// Row label; matches `benches/encode.rs`, plus a `+clean` suffix where this row differs from
    /// its neighbour only by [`PngEncoder::with_transparent_cleanup`].
    name: &'static str,
    /// Corpus generator key. Distinct from `name` so a cleaned row can share a fixture with its
    /// uncleaned twin rather than duplicating the pixels.
    fixture: &'static str,
    /// The square side to measure at.
    side: u32,
    /// Whether to enable [`PngEncoder::with_transparent_cleanup`].
    cleanup: bool,
    /// The most gamut's file may measure as a fraction of libpng's. `1.00` reads "never larger".
    ///
    /// Derived, not chosen: `measured × (1 + headroom)` rounded up to two decimals, where the
    /// headroom is 5% unless this row's `why` names the other component whose drift it absorbs.
    max_ratio: f64,
    /// What the case actually measures at this revision, so drift is visible in review.
    ///
    /// To refresh the whole table after an encoder change: set every `max_ratio` to `2.00`, run
    /// `cargo test -p gamut-png --test size_contract
    /// gamut_never_exceeds_its_size_budget_against_libpng9 -- --exact --nocapture`, paste each
    /// printed ratio back into `measured`, then re-derive `max_ratio` by the rule above.
    measured: f64,
    /// Why this number and not a tighter one — which stage spends the bytes.
    why: &'static str,
}

/// Every budget carries its justification, and every `max_ratio` is derived from the `measured`
/// beside it rather than chosen -- see [`Budget::max_ratio`].
///
/// Measured at 128x128 (a quarter of the bench's pixel count, so the suite stays quick enough for
/// the coverage and mutation lanes) except `tiny_rgb8`, which is the bench's own 16x16 row.
///
/// These ratios are **not** comparable with the bench's 256x256 figures and must be read
/// separately. Every fixed cost -- the signature, IHDR, PLTE/tRNS, IEND, and DEFLATE's own framing
/// -- is amortised over a quarter as many pixels here, which systematically disadvantages exactly
/// the rows where a reduction wins: the gap runs to about 30 percentage points on `gradient_rgb8`
/// and `palette64_rgba8`. `STATUS.md` records the 256x256 table; this one gates.
const BUDGETS: &[Budget] = &[
    Budget {
        name: "gradient_rgb8",
        fixture: "gradient_rgb8",
        side: 128,
        cleanup: false,
        max_ratio: 0.82,
        measured: 0.772,
        why: "no reduction applies, so this is filtering plus DEFLATE against libpng's own \
              adaptive filtering. The margin is thin by nature -- both encoders are doing the \
              same job -- so the budget only guards against losing outright.",
    },
    Budget {
        name: "photo_rgb8",
        fixture: "photo_rgb8",
        side: 128,
        cleanup: false,
        max_ratio: 0.83,
        measured: 0.731,
        why: "smooth photographic content: palette-hostile, so again pure filtering + DEFLATE, \
              and the win is the optimal parse. Coupled to gamut-deflate's own Best/z9 column by \
              construction: if that regresses, this row moves with it, so it carries 13% headroom \
              where the others carry 5%.",
    },
    Budget {
        name: "noise_rgb8",
        fixture: "noise_rgb8",
        side: 128,
        cleanup: false,
        max_ratio: 1.02,
        measured: 0.998,
        why: "incompressible, so both encoders fall back to stored blocks and the file is \
              slightly larger than the raw samples. Above 1.0 because there is nothing to win \
              here, not because we lose; the 2% margin covers stored-block framing only.",
    },
    Budget {
        name: "grey_as_rgb8",
        fixture: "grey_as_rgb8",
        side: 128,
        cleanup: false,
        max_ratio: 0.62,
        measured: 0.582,
        why: "R=G=B everywhere, so auto-reduce drops two channels before DEFLATE runs. A \
              structural win libpng does not attempt.",
    },
    Budget {
        name: "flat_rgba8",
        fixture: "flat_rgba8",
        side: 128,
        cleanup: false,
        max_ratio: 0.36,
        measured: 0.321,
        why: "one opaque colour: the reduce cascade collapses it to depth-1 indexed, and chunk \
              framing is most of what remains. 10% headroom because at ~100 bytes total a single \
              byte moves the ratio by about a percent.",
    },
    Budget {
        name: "sprite_rgba8",
        fixture: "sprite_rgba8",
        side: 128,
        cleanup: false,
        max_ratio: 0.99,
        measured: 0.963,
        why: "binary alpha over invisible colour noise. The reduce cascade now reaches this \
              case -- `write_reduced_or_native` races an `RGB`+`tRNS` colour key against the \
              unreduced encoding and keeps whichever is smaller -- so the budget is a real one \
              rather than the placeholder 1.00 it carried while those axes were missing. \
              Tightening it was #481's stated acceptance test. 2% headroom, not 5%: at 0.963 the \
              usual 5% rounds past 1.00, which would give up the very claim this row exists to \
              make.",
    },
    Budget {
        name: "sprite_rgba8 +clean",
        fixture: "sprite_rgba8",
        side: 128,
        cleanup: true,
        max_ratio: 0.70,
        measured: 0.665,
        why: "the same pixels with `with_transparent_cleanup`, which collapses every invisible \
              pixel to one colour and so makes the palette reachable. This is the row that gates \
              the `+clean` column STATUS.md publishes; without it the headline cleanup result was \
              measured by a bench and asserted by nothing.",
    },
    Budget {
        name: "palette64_rgba8",
        fixture: "palette64_rgba8",
        side: 128,
        cleanup: false,
        max_ratio: 0.95,
        measured: 0.899,
        why: "64 colours over two alpha levels. The palette encoding wins outright at 256x256 \
              but loses at this size, because PLTE + tRNS is a flat 273 incompressible bytes \
              against pixels that compress ~160x; `write_reduced_or_native` encodes both and \
              keeps the smaller, so the row measures whichever is actually better here. The race \
              is what makes the outcome stable enough to budget below 1.00.",
    },
    Budget {
        name: "palette64_rgba8 +clean",
        fixture: "palette64_rgba8",
        side: 128,
        cleanup: true,
        max_ratio: 1.02,
        measured: 0.995,
        why: "cleaning *costs* bytes here -- 403 against the uncleaned row's 364 -- and that is \
              the point of the row. Collapsing the transparent entries does shorten PLTE and \
              tRNS, but it also rewrites pixels that were compressing well, and at 128x128 the \
              second effect wins. `with_transparent_cleanup` is a canonicalisation, not an \
              optimisation, and this is the case that says so out loud; the same trade is \
              asserted directly by `a_colour_key_can_lose_the_size_race`. 2% headroom for the \
              same reason as `noise_rgb8`: there is no win here to protect.",
    },
    Budget {
        name: "tiny_rgb8",
        fixture: "tiny_rgb8",
        side: 16,
        cleanup: false,
        max_ratio: 0.95,
        measured: 0.862,
        why: "the regime where the signature and five chunks of framing dominate, and the only \
              row where `overhead_bytes` is legible. Reported by the bench and, until now, gated \
              by nothing. Same 10% headroom as `flat_rgba8`, for the same reason.",
    },
];

/// Half the bench's side, so this file stays fast enough for the coverage and mutation lanes.
const SIDE: u32 = 128;

/// The pixels for a budget row, and how many channels they carry.
fn pixels(fixture: &str, side: u32) -> (Vec<u8>, usize) {
    match fixture {
        "gradient_rgb8" => (common::corpus::gradient_rgb(side), 3),
        "photo_rgb8" => (common::corpus::photo_rgb(side), 3),
        "noise_rgb8" => (common::corpus::noise_rgb(side), 3),
        "grey_as_rgb8" => (common::corpus::grey_as_rgb(side), 3),
        "palette64_rgba8" => (common::corpus::palette64_rgba(side), 4),
        "sprite_rgba8" => (common::corpus::sprite_rgba(side), 4),
        "flat_rgba8" => (common::corpus::flat_rgba(side), 4),
        // The bench's 16x16 row: the regime where chunk framing dominates bits-per-pixel.
        "tiny_rgb8" => (common::corpus::gradient_rgb(side), 3),
        other => panic!("unknown corpus fixture {other}"),
    }
}

/// Encodes at the crate's smallest-output settings.
///
/// `BruteForce`'s candidate set is integer-only -- `MinEntropy` is deliberately not in it -- so no
/// `f64::log2` enters the gated path and these ratios are machine-independent as well as stable
/// run to run.
fn gamut_best(samples: &[u8], channels: usize, side: u32, cleanup: bool) -> Vec<u8> {
    let encoder = PngEncoder::new()
        .with_compression(Level::Best)
        .with_filter(FilterStrategy::BruteForce)
        .with_auto_reduce(true)
        .with_transparent_cleanup(cleanup);
    let dims = Dimensions::new(side, side).expect("valid dimensions");
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
fn libpng9(samples: &[u8], channels: usize, side: u32) -> Vec<u8> {
    let color_type = if channels == 3 {
        libpng_oracle::COLOR_RGB
    } else {
        libpng_oracle::COLOR_RGBA
    };
    libpng_oracle::encode(
        samples,
        side,
        side,
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
        let (samples, channels) = pixels(budget.fixture, budget.side);
        let ours = gamut_best(&samples, channels, budget.side, budget.cleanup);
        let theirs = libpng9(&samples, channels, budget.side);
        let ratio = ours.len() as f64 / theirs.len() as f64;
        // Printed, not just asserted: the `measured` column is only honest if refreshing it is a
        // paste rather than a re-derivation. `cargo test` captures this on success.
        println!(
            "{:<22} {:>7} / {:>7} = {ratio:.3}   (budget {:.2}, recorded {:.3})",
            budget.name,
            ours.len(),
            theirs.len(),
            budget.max_ratio,
            budget.measured,
        );
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
        "palette64_rgba8",
    ];
    for budget in BUDGETS.iter().filter(|b| WINS.contains(&b.name)) {
        let (samples, channels) = pixels(budget.fixture, budget.side);
        let ours = gamut_best(&samples, channels, budget.side, budget.cleanup);
        let theirs = libpng9(&samples, channels, budget.side);
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
        let (samples, channels) = pixels(name, SIDE);
        let ours = gamut_best(&samples, channels, SIDE, false);
        let theirs = libpng9(&samples, channels, SIDE);
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
        let (samples, channels) = pixels(budget.fixture, budget.side);
        let first = gamut_best(&samples, channels, budget.side, budget.cleanup);
        let second = gamut_best(&samples, channels, budget.side, budget.cleanup);
        assert_eq!(first, second, "{}: encode is not reproducible", budget.name);
    }
}

