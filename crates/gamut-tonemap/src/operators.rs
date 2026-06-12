//! Built-in tone-mapping operators implementing [`ToneCurve`](crate::curve::ToneCurve).
//!
//! Every operator maps non-negative linear-light input to a tone-mapped output. Parameterless
//! operators ([`Linear`], [`Reinhard`]) are zero-sized; parameterized operators ([`Clamp`],
//! [`ReinhardExtended`]) validate their parameter at construction so an out-of-range value can
//! never reach [`map`](crate::curve::ToneCurve::map).

use gamut_core::{Error, Result};

use crate::constants::{DEFAULT_REINHARD_WHITE, SDR_WHITE_NORMALIZED};
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Absolute tolerance for f32 comparisons of derived (non-exact) values.
    const EPS: f32 = 1e-6;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() <= EPS
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
}
