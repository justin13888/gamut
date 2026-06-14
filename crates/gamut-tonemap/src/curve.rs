//! The [`ToneCurve`] trait — the extension point for tone-mapping operators.

/// A tone-mapping operator: a map from a non-negative linear-light input to a tone-mapped output.
///
/// Built-in operators live in [`operators`](crate::operators) and are re-exported at the crate
/// root; implement this trait yourself to supply a custom curve, or rely on the blanket impl that
/// makes any `Fn(f32) -> f32` a `ToneCurve` so a closure works directly.
///
/// Implementors provide only [`map`](ToneCurve::map); [`map_slice`](ToneCurve::map_slice) is
/// derived from it.
///
/// # Contract
///
/// For a finite, non-negative input, every built-in operator returns a finite, non-negative output
/// and is monotonic non-decreasing in `x`. Behaviour on negative or NaN inputs is operator-defined
/// and outside this contract — linearize and clamp upstream if a source can produce them.
pub trait ToneCurve {
    /// Map a single non-negative linear-light value `x` to its tone-mapped output.
    ///
    /// `x` is assumed finite and `>= 0` (see the trait-level contract); behaviour on negative or
    /// NaN inputs is operator-defined.
    #[must_use]
    fn map(&self, x: f32) -> f32;

    /// Apply [`map`](ToneCurve::map) to every element of `buf` in place.
    ///
    /// Each element is mapped independently, so the result is unaffected by `buf`'s length or the
    /// order of its elements.
    fn map_slice(&self, buf: &mut [f32]) {
        for x in buf {
            *x = self.map(*x);
        }
    }
}

/// Any `Fn(f32) -> f32` is a [`ToneCurve`], so a closure or function pointer is usable directly as
/// a curve without defining a new type.
impl<F: Fn(f32) -> f32> ToneCurve for F {
    fn map(&self, x: f32) -> f32 {
        self(x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_slice_applies_map_to_every_element() {
        // A simple doubling closure stands in for any curve.
        let curve = |x: f32| x * 2.0;
        let mut buf = [0.0_f32, 1.5, 3.0, -2.0];
        let expected: [f32; 4] = core::array::from_fn(|i| curve.map(buf[i]));
        curve.map_slice(&mut buf);
        assert_eq!(buf, expected);
    }

    #[test]
    fn closure_is_a_tone_curve() {
        let gamma = |x: f32| x.powf(1.0 / 2.2);
        assert_eq!(gamma.map(1.0), 1.0);
        assert!(gamma.map(0.5) > 0.5);

        // Also usable behind a trait object.
        let c: &dyn ToneCurve = &gamma;
        assert_eq!(c.map(0.0), 0.0);
    }
}
