//! Built-in tone-mapping operators implementing [`ToneCurve`].
//!
//! Every operator maps non-negative linear-light input to a tone-mapped output. Parameterless
//! operators ([`Linear`], [`Reinhard`], [`Aces`]) are zero-sized; parameterized operators
//! ([`Clamp`], [`Exposure`], [`ReinhardExtended`], [`Hable`], [`Drago`]) validate their parameters
//! at construction so an out-of-range value can never reach
//! [`map`](crate::curve::ToneCurve::map). Each operator's exact formula and primary source are
//! recorded in `references/tonemap/README.md`.

use gamut_core::{Error, Result};

use crate::constants::{
    DEFAULT_DRAGO_BIAS, DEFAULT_HABLE_WHITE, DEFAULT_REINHARD_WHITE, SDR_WHITE_NORMALIZED,
};
use crate::curve::ToneCurve;

/// Identity passthrough: `map(x) == x`.
///
/// Useful as a no-op default in a generic pipeline parameterized over a [`ToneCurve`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Linear;

impl ToneCurve for Linear {
    fn map(&self, x: f32) -> f32 {
        x
    }
}

/// Hard-clamp curve: `map(x) == x.clamp(0.0, max)`.
///
/// The simplest way to fit an out-of-range signal into `[0, max]`; unlike the Reinhard operators
/// it preserves values below `max` exactly and flatly discards everything above.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clamp {
    max: f32,
}

impl Clamp {
    /// Create a clamp curve with upper bound `max`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `max` is not finite or is not strictly positive.
    pub fn new(max: f32) -> Result<Self> {
        if !max.is_finite() || max <= 0.0 {
            return Err(Error::InvalidInput(
                "Clamp max must be finite and greater than zero",
            ));
        }
        Ok(Self { max })
    }

    /// The upper bound this curve clamps to.
    #[must_use]
    pub fn max(self) -> f32 {
        self.max
    }
}

impl Default for Clamp {
    fn default() -> Self {
        Self {
            max: SDR_WHITE_NORMALIZED,
        }
    }
}

impl ToneCurve for Clamp {
    fn map(&self, x: f32) -> f32 {
        x.clamp(0.0, self.max)
    }
}

/// Reinhard tone mapping: `map(x) == x / (1 + x)`.
///
/// The canonical parameterless global operator; maps `[0, ∞)` into `[0, 1)`. Highlights roll off
/// smoothly but never quite reach `1.0`, so very bright detail is compressed rather than clipped.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reinhard;

impl ToneCurve for Reinhard {
    fn map(&self, x: f32) -> f32 {
        x / (1.0 + x)
    }
}

/// Extended Reinhard with a white point: `map(x) == x * (1 + x / white²) / (1 + x)`.
///
/// `white` is the smallest linear input that maps exactly to `1.0`; inputs above it are pushed
/// past `1.0`, so callers typically pick `white` at or above the brightest sample. As
/// `white → ∞` the operator reduces to plain [`Reinhard`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReinhardExtended {
    white: f32,
}

impl ReinhardExtended {
    /// Create an extended-Reinhard curve whose white point is `white`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `white` is not finite or is not strictly positive.
    pub fn new(white: f32) -> Result<Self> {
        if !white.is_finite() || white <= 0.0 {
            return Err(Error::InvalidInput(
                "Reinhard white point must be finite and greater than zero",
            ));
        }
        Ok(Self { white })
    }

    /// The white point: the linear input that maps to display white (`1.0`).
    #[must_use]
    pub fn white(self) -> f32 {
        self.white
    }
}

impl Default for ReinhardExtended {
    fn default() -> Self {
        Self {
            white: DEFAULT_REINHARD_WHITE,
        }
    }
}

impl ToneCurve for ReinhardExtended {
    fn map(&self, x: f32) -> f32 {
        let numerator = x * (1.0 + x / (self.white * self.white));
        numerator / (1.0 + x)
    }
}

/// Exposure pre-scaling: `map(x) == x * scale`.
///
/// A composable linear gain applied before another curve — the explicit form of the exposure /
/// key-value step several operators fold in (Reinhard's key scaling, the ACES `×0.6` input,
/// Hable's `×2.0` bias). There is no `Default`: there is no canonical exposure — use [`Linear`] for
/// a no-op. See `references/tonemap/README.md`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Exposure {
    scale: f32,
}

impl Exposure {
    /// Create an exposure curve with linear gain `scale`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `scale` is not finite or is not strictly positive.
    pub fn new(scale: f32) -> Result<Self> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(Error::InvalidInput(
                "Exposure scale must be finite and greater than zero",
            ));
        }
        Ok(Self { scale })
    }

    /// Create an exposure curve from a photographic stop count: `scale == 2^stops` (one stop
    /// doubles exposure).
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `2^stops` is not a finite, strictly positive gain (i.e.
    /// `stops` is NaN or large enough to overflow).
    pub fn from_stops(stops: f32) -> Result<Self> {
        Self::new(stops.exp2())
    }

    /// The linear gain this curve multiplies by.
    #[must_use]
    pub fn scale(self) -> f32 {
        self.scale
    }
}

impl ToneCurve for Exposure {
    fn map(&self, x: f32) -> f32 {
        x * self.scale
    }
}

/// ACES filmic tone-mapping curve — the Narkowicz (2016) approximation:
/// `saturate((x·(2.51x + 0.03)) / (x·(2.43x + 0.59) + 0.14))`.
///
/// A luminance-only rational fit to the ACES `ODT(RRT(x))` for Rec.709/D65 output; **not** the full
/// ACES transform (which is colour-space-coupled and out of scope for a scalar curve). Maps
/// `[0, ∞) → [0, 1]`. The input is taken as pre-exposed (so `1 → ≈0.8`); apply
/// [`Exposure::new`]`(0.6)` first to match the original ACES curve. See
/// `references/tonemap/README.md`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Aces;

impl ToneCurve for Aces {
    fn map(&self, x: f32) -> f32 {
        // Narkowicz 2016 coefficients (see references/tonemap/README.md).
        const A: f32 = 2.51;
        const B: f32 = 0.03;
        const C: f32 = 2.43;
        const D: f32 = 0.59;
        const E: f32 = 0.14;
        ((x * (A * x + B)) / (x * (C * x + D) + E)).clamp(0.0, 1.0)
    }
}

// Hable / Uncharted 2 filmic constants (John Hable, 2010; see references/tonemap/README.md).
const HABLE_A: f32 = 0.15; // shoulder strength
const HABLE_B: f32 = 0.50; // linear strength
const HABLE_C: f32 = 0.10; // linear angle
const HABLE_D: f32 = 0.20; // toe strength
const HABLE_E: f32 = 0.02; // toe numerator
const HABLE_F: f32 = 0.30; // toe denominator

/// The un-normalized Uncharted 2 partial curve `partial(x)` (see [`Hable`]).
fn hable_partial(x: f32) -> f32 {
    ((x * (HABLE_A * x + HABLE_C * HABLE_B) + HABLE_D * HABLE_E)
        / (x * (HABLE_A * x + HABLE_B) + HABLE_D * HABLE_F))
        - HABLE_E / HABLE_F
}

/// Hable / Uncharted 2 filmic operator (John Hable, 2010), white-point normalized so
/// `map(white) == 1` and `map(0) == 0`.
///
/// `map(x) = partial(x) / partial(white)` with the published shoulder/toe constants. The original
/// presentation additionally applied a `×2.0` exposure bias (compose [`Exposure::new`]`(2.0)`) and a
/// final `pow(·, 1/2.2)` display encode (the target transfer function's job, e.g. via
/// `gamut-color`) — both excluded here. See `references/tonemap/README.md`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hable {
    white: f32,
}

impl Hable {
    /// Create a Hable curve whose linear white point is `white` (`map(white) == 1`).
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `white` is not finite or is not strictly positive.
    pub fn new(white: f32) -> Result<Self> {
        if !white.is_finite() || white <= 0.0 {
            return Err(Error::InvalidInput(
                "Hable white point must be finite and greater than zero",
            ));
        }
        Ok(Self { white })
    }

    /// The linear white point: the input that maps to display white (`1.0`).
    #[must_use]
    pub fn white(self) -> f32 {
        self.white
    }
}

impl Default for Hable {
    fn default() -> Self {
        Self {
            white: DEFAULT_HABLE_WHITE,
        }
    }
}

impl ToneCurve for Hable {
    fn map(&self, x: f32) -> f32 {
        hable_partial(x) / hable_partial(self.white)
    }
}

// Drago et al. 2003 reference display max (cd/m²); `Ldmax · 0.01 = 1` normalizes output to [0, 1].
const DRAGO_LDMAX_NITS: f32 = 100.0;

/// Drago et al. (2003) adaptive logarithmic operator, mapping scene luminance to display-relative
/// `[0, 1]` (`Ldmax` normalized to 100 cd/m²).
///
/// Needs the scene's maximum luminance, so there is no `Default`. `map(0) == 0` and
/// `map(world_max) == 1`. For `bias < 0.7` the output may exceed `1.0` (the display clamps to
/// `Ldmax`); `map` is a faithful, clamp-free transcription of the paper's Eq. (4). See
/// `references/tonemap/README.md`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drago {
    world_max: f32,
    bias: f32,
}

impl Drago {
    /// Create a Drago curve for a scene whose maximum luminance is `world_max`, with the default
    /// bias ([`DEFAULT_DRAGO_BIAS`]). Tune it with [`Drago::with_bias`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `world_max` is not finite or is not strictly positive.
    pub fn new(world_max: f32) -> Result<Self> {
        if !world_max.is_finite() || world_max <= 0.0 {
            return Err(Error::InvalidInput(
                "Drago world_max must be finite and greater than zero",
            ));
        }
        Ok(Self {
            world_max,
            bias: DEFAULT_DRAGO_BIAS,
        })
    }

    /// Set the bias, which steers contrast in dark vs. bright regions; the paper's useful range is
    /// `(0, 1)`, default `0.85`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `bias` is not finite or is not in the open interval
    /// `(0, 1)`.
    pub fn with_bias(self, bias: f32) -> Result<Self> {
        if !bias.is_finite() || bias <= 0.0 || bias >= 1.0 {
            return Err(Error::InvalidInput(
                "Drago bias must be finite and in the open interval (0, 1)",
            ));
        }
        Ok(Self { bias, ..self })
    }

    /// The scene's maximum luminance.
    #[must_use]
    pub fn world_max(self) -> f32 {
        self.world_max
    }

    /// The bias parameter.
    #[must_use]
    pub fn bias(self) -> f32 {
        self.bias
    }
}

impl ToneCurve for Drago {
    fn map(&self, x: f32) -> f32 {
        // Drago et al. 2003 Eq. (4) with the Eq. (3) bias (see references/tonemap/README.md).
        let exponent = self.bias.ln() / 0.5_f32.ln();
        let ratio = (x / self.world_max).powf(exponent);
        let denom = (2.0 + ratio * 8.0).ln();
        let scale = (DRAGO_LDMAX_NITS * 0.01) / (self.world_max + 1.0).log10();
        scale * (x + 1.0).ln() / denom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Absolute tolerance for f32 comparisons of derived (non-exact) values.
    const EPS: f32 = 1e-6;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() <= EPS
    }

    fn close_eps(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    /// Assert `curve` is strictly increasing across a sampled grid (kills sign/operator mutants).
    fn assert_monotonic_increasing(curve: &impl ToneCurve, start: f32, step: f32, samples: u32) {
        let mut prev = curve.map(start);
        for i in 1..=samples {
            let y = curve.map(start + i as f32 * step);
            assert!(y > prev, "not increasing at sample {i}");
            prev = y;
        }
    }

    #[test]
    fn linear_is_identity() {
        let c = Linear;
        for x in [0.0, 0.5, 1.0, 42.0, 1e6] {
            assert_eq!(c.map(x), x);
        }
    }

    #[test]
    fn clamp_bounds_input() {
        let c = Clamp::new(1.0).expect("1.0 is a valid bound");
        assert_eq!(c.map(-1.0), 0.0);
        assert_eq!(c.map(0.5), 0.5);
        assert_eq!(c.map(2.0), 1.0);
        assert_eq!(c.max(), 1.0);
        assert_eq!(Clamp::default().max(), SDR_WHITE_NORMALIZED);
        // A non-1.0 bound pins `max()`: the cases above all equal 1.0, which a constant-1.0 mutant
        // also returns.
        let c2 = Clamp::new(2.5).expect("valid bound");
        assert_eq!(c2.max(), 2.5);
        assert_eq!(c2.map(3.0), 2.5);
    }

    #[test]
    fn clamp_rejects_invalid_max() {
        assert!(Clamp::new(0.0).is_err());
        assert!(Clamp::new(-1.0).is_err());
        assert!(Clamp::new(f32::NAN).is_err());
        assert!(Clamp::new(f32::INFINITY).is_err());
    }

    #[test]
    fn reinhard_fixed_points_and_monotonic() {
        let c = Reinhard;
        assert_eq!(c.map(0.0), 0.0);
        assert_eq!(c.map(1.0), 0.5);

        // Strictly increasing and bounded below 1 over a sampled grid.
        let mut prev = c.map(0.0);
        for i in 1..=1000 {
            let y = c.map(i as f32 * 0.1);
            assert!(y > prev, "not increasing at step {i}");
            assert!(y < 1.0, "not bounded below 1 at step {i}");
            prev = y;
        }

        // Asymptotically approaches 1 from below.
        assert!(c.map(1e7) < 1.0);
        assert!(close(c.map(1e7), 1.0));
    }

    #[test]
    fn reinhard_extended_white_point_and_convergence() {
        let white = 4.0;
        let c = ReinhardExtended::new(white).expect("positive white point");
        assert_eq!(c.map(0.0), 0.0);
        assert!(close(c.map(white), 1.0));
        assert_eq!(c.white(), white);

        // Monotonic increasing on a sampled grid.
        let mut prev = c.map(0.0);
        for i in 1..=1000 {
            let y = c.map(i as f32 * 0.01);
            assert!(y > prev, "not increasing at step {i}");
            prev = y;
        }

        // As white → ∞ it converges to plain Reinhard.
        let wide = ReinhardExtended::new(1e6).expect("positive white point");
        let plain = Reinhard;
        for x in [0.1, 1.0, 5.0, 50.0] {
            assert!(close(wide.map(x), plain.map(x)), "mismatch at {x}");
        }

        // The default white point is the documented HDR/SDR reference ratio.
        assert!(close(
            ReinhardExtended::default().white(),
            DEFAULT_REINHARD_WHITE
        ));
        // Pin the ratio's literal value (203 / 100 = 2.03), not just the self-referential default,
        // so a mutated `/` in the constant is observable.
        assert!(close(DEFAULT_REINHARD_WHITE, 2.03));
    }

    #[test]
    fn reinhard_extended_rejects_invalid_white() {
        assert!(ReinhardExtended::new(0.0).is_err());
        assert!(ReinhardExtended::new(-2.0).is_err());
        assert!(ReinhardExtended::new(f32::NAN).is_err());
        assert!(ReinhardExtended::new(f32::INFINITY).is_err());
    }

    #[test]
    fn operators_are_object_safe() {
        let curves: [&dyn ToneCurve; 4] = [
            &Linear,
            &Clamp::default(),
            &Reinhard,
            &ReinhardExtended::default(),
        ];
        for c in curves {
            // Zero maps to a finite, non-negative value for every built-in operator.
            let y = c.map(0.0);
            assert!(y.is_finite() && y >= 0.0);
        }
    }

    #[test]
    fn exposure_scales_by_gain() {
        let c = Exposure::new(2.0).expect("positive gain");
        assert_eq!(c.map(0.0), 0.0);
        assert_eq!(c.map(3.0), 6.0);
        assert_eq!(c.scale(), 2.0);
        // from_stops: one stop doubles, zero stops is unity, minus one stop halves.
        assert_eq!(Exposure::from_stops(1.0).expect("finite").scale(), 2.0);
        assert_eq!(Exposure::from_stops(0.0).expect("finite").scale(), 1.0);
        assert_eq!(Exposure::from_stops(-1.0).expect("finite").scale(), 0.5);
    }

    #[test]
    fn exposure_rejects_invalid() {
        assert!(Exposure::new(0.0).is_err());
        assert!(Exposure::new(-1.0).is_err());
        assert!(Exposure::new(f32::NAN).is_err());
        assert!(Exposure::new(f32::INFINITY).is_err());
        assert!(Exposure::from_stops(f32::NAN).is_err());
        assert!(Exposure::from_stops(f32::INFINITY).is_err()); // 2^inf = inf
    }

    #[test]
    fn aces_fixed_points_and_reference() {
        let c = Aces;
        assert_eq!(c.map(0.0), 0.0);
        // Independent golden values for the Narkowicz curve (see references/tonemap/README.md).
        assert!(close_eps(c.map(0.5), 0.616_307, 1e-5));
        assert!(close_eps(c.map(1.0), 0.803_797, 1e-5));
        // Saturates to 1 for large inputs (unclamped limit a/c ≈ 1.033).
        assert_eq!(c.map(1e6), 1.0);
        // Strictly increasing below saturation (the curve reaches 1.0 near x ≈ 7.24).
        assert_monotonic_increasing(&c, 0.01, 0.01, 200);
    }

    #[test]
    fn hable_fixed_points_and_reference() {
        let c = Hable::default();
        assert_eq!(c.white(), DEFAULT_HABLE_WHITE);
        // partial(0) cancels to ~0 in exact math; f32 leaves a sub-ULP residual.
        assert!(close_eps(c.map(0.0), 0.0, 1e-6));
        assert_eq!(c.map(c.white()), 1.0); // partial(W)/partial(W) is exactly 1
        // Independent golden values at W = 11.2 (see references/tonemap/README.md).
        assert!(close_eps(c.map(1.0), 0.304_300, 1e-4));
        assert!(close_eps(c.map(4.0), 0.713_240, 1e-4));
        assert_monotonic_increasing(&c, 0.05, 0.05, 220);
        // A custom white point pins white() and the unit-output input.
        let c2 = Hable::new(6.0).expect("positive white");
        assert_eq!(c2.white(), 6.0);
        assert_eq!(c2.map(6.0), 1.0);
    }

    #[test]
    fn hable_rejects_invalid_white() {
        assert!(Hable::new(0.0).is_err());
        assert!(Hable::new(-1.0).is_err());
        assert!(Hable::new(f32::NAN).is_err());
        assert!(Hable::new(f32::INFINITY).is_err());
    }

    #[test]
    fn drago_fixed_points_and_reference() {
        let c = Drago::new(100.0).expect("positive world max");
        assert_eq!(c.world_max(), 100.0);
        assert_eq!(c.bias(), DEFAULT_DRAGO_BIAS);
        assert_eq!(c.map(0.0), 0.0);
        assert!(close_eps(c.map(c.world_max()), 1.0, 1e-5));
        // Independent golden value at world_max = 100, bias = 0.85 (see references/tonemap/README.md).
        assert!(close_eps(c.map(10.0), 0.630_858, 1e-4));
        assert_monotonic_increasing(&c, 0.1, 0.1, 500);
        // bias = 0.5 makes the bias exponent exactly 1; the world-max fixed point still holds.
        let c2 = c.with_bias(0.5).expect("bias in (0,1)");
        assert_eq!(c2.bias(), 0.5);
        assert!(close_eps(c2.map(c2.world_max()), 1.0, 1e-5));
    }

    #[test]
    fn drago_rejects_invalid_params() {
        assert!(Drago::new(0.0).is_err());
        assert!(Drago::new(-1.0).is_err());
        assert!(Drago::new(f32::NAN).is_err());
        assert!(Drago::new(f32::INFINITY).is_err());
        let c = Drago::new(100.0).expect("valid world max");
        assert!(c.with_bias(0.0).is_err());
        assert!(c.with_bias(1.0).is_err());
        assert!(c.with_bias(-0.1).is_err());
        assert!(c.with_bias(1.5).is_err());
        assert!(c.with_bias(f32::NAN).is_err());
    }
}
