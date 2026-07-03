//! Tone reproduction curves: `curveType` and `parametricCurveType` (ICC.1:2022 §10.6, §10.18).
//!
//! Both model a 1-D function from `[0, 1]` to `[0, 1]`; [`Curve::eval`] / [`ParametricCurve::eval`]
//! evaluate them, which is also the bridge for cross-checking against a reference CMM.

use gamut_core::{Error, Result};

use crate::bytes::{ByteReader, pad_to_4, push_s15fixed16};
use crate::primitives::{S15Fixed16, U8Fixed8};

/// A one-dimensional tone curve (`curveType`, ICC.1:2022 §10.6).
///
/// The on-disk count field selects the encoding: zero entries is the identity, one entry is a
/// `u8Fixed8` gamma, and two or more entries are a uniformly-sampled table over `[0, 1]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Curve {
    /// Zero entries: the identity curve, `Y = X`.
    Identity,
    /// One entry: a pure-gamma curve `Y = X^g`, with `g` a `u8Fixed8`.
    Gamma(U8Fixed8),
    /// Two or more entries: a uniformly-spaced lookup over `[0, 1]`, each sample a `u16` scaled by
    /// `65535`.
    Sampled(Vec<u16>),
}

impl Curve {
    /// Evaluates the curve at `x`, clamping the input and output to `[0, 1]`.
    #[must_use]
    pub fn eval(&self, x: f64) -> f64 {
        let x = x.clamp(0.0, 1.0);
        match self {
            Curve::Identity => x,
            Curve::Gamma(g) => x.powf(g.to_f64()),
            Curve::Sampled(table) => sample_table(table, x),
        }
    }
}

/// Linearly interpolates a uniformly-spaced sample table at `x` in `[0, 1]`.
fn sample_table(table: &[u16], x: f64) -> f64 {
    match table.len() {
        0 => x,
        1 => f64::from(table[0]) / 65535.0,
        n => {
            let pos = x * (n - 1) as f64;
            let lower = pos.floor() as usize;
            if lower >= n - 1 {
                return f64::from(table[n - 1]) / 65535.0;
            }
            let frac = pos - lower as f64;
            let a = f64::from(table[lower]);
            let b = f64::from(table[lower + 1]);
            (a + (b - a) * frac) / 65535.0
        }
    }
}

/// A parametric tone curve (`parametricCurveType`, ICC.1:2022 §10.18).
///
/// The `function_type` selects one of five closed forms (0–4) over the parameters, in the spec's
/// order `g, a, b, c, d, e, f`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParametricCurve {
    /// The function type (0–4).
    pub function_type: u16,
    /// The parameters as `s15Fixed16`, in the order `g, a, b, c, d, e, f` (only the leading ones the
    /// function type requires are present).
    pub params: Vec<S15Fixed16>,
}

impl ParametricCurve {
    /// Evaluates the curve at `x`, clamping the input to `[0, 1]`.
    ///
    /// Implements the five defined function types (ICC.1:2022 §10.18); an unrecognized function type
    /// evaluates as the identity.
    #[must_use]
    pub fn eval(&self, x: f64) -> f64 {
        let x = x.clamp(0.0, 1.0);
        let p = |i: usize, default: f64| self.params.get(i).map_or(default, |v| v.to_f64());
        let (g, a, b, c, d, e, f) = (
            p(0, 1.0),
            p(1, 1.0),
            p(2, 0.0),
            p(3, 0.0),
            p(4, 0.0),
            p(5, 0.0),
            p(6, 0.0),
        );
        // `(aX + b)^g`, clamping the base to keep `powf` real. For types 1 and 2 this clamp also
        // realizes the spec's `else 0` branch (when `aX + b < 0`), so they need no explicit
        // condition; types 3 and 4 switch to a separate linear segment below the threshold `d`.
        let power = |base: f64| base.max(0.0).powf(g);
        match self.function_type {
            0 => x.powf(g),
            1 => power(a * x + b),
            2 => power(a * x + b) + c,
            3 => {
                if x >= d {
                    power(a * x + b)
                } else {
                    c * x
                }
            }
            4 => {
                if x >= d {
                    power(a * x + b) + e
                } else {
                    c * x + f
                }
            }
            _ => x,
        }
    }
}

/// Either kind of tone curve, as carried by the curve sets inside the LUT transform types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CurveOrParametric {
    /// A `curveType` curve.
    Curve(Curve),
    /// A `parametricCurveType` curve.
    Parametric(ParametricCurve),
}

impl CurveOrParametric {
    /// Evaluates the curve at `x` in `[0, 1]`.
    #[must_use]
    pub fn eval(&self, x: f64) -> f64 {
        match self {
            CurveOrParametric::Curve(curve) => curve.eval(x),
            CurveOrParametric::Parametric(curve) => curve.eval(x),
        }
    }
}

/// The number of parameters a `parametricCurveType` function type carries (ICC.1:2022 §10.18).
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] for function types outside the defined range 0–4.
pub(crate) fn parametric_param_count(function_type: u16) -> Result<usize> {
    Ok(match function_type {
        0 => 1,
        1 => 3,
        2 => 4,
        3 => 5,
        4 => 7,
        _ => {
            return Err(Error::InvalidInput("icc: invalid parametric function type"));
        }
    })
}

/// Reads a `curveType` body (the count and entries) from `r`, positioned just after the element's
/// type signature and reserved bytes.
pub(crate) fn read_curve_body(r: &mut ByteReader<'_>) -> Result<Curve> {
    let count = r.u32()? as usize;
    Ok(match count {
        0 => Curve::Identity,
        1 => Curve::Gamma(U8Fixed8(r.u16()?)),
        n => {
            if n.checked_mul(2).is_none_or(|bytes| bytes > r.remaining()) {
                return Err(Error::InvalidInput("icc: curve table exceeds element"));
            }
            let mut table = Vec::with_capacity(n);
            for _ in 0..n {
                table.push(r.u16()?);
            }
            Curve::Sampled(table)
        }
    })
}

/// Reads a `parametricCurveType` body (the function type and its parameters) from `r`, positioned
/// just after the element's type signature and reserved bytes.
pub(crate) fn read_parametric_body(r: &mut ByteReader<'_>) -> Result<ParametricCurve> {
    let function_type = r.u16()?;
    r.skip(2)?; // reserved
    let count = parametric_param_count(function_type)?;
    let mut params = Vec::with_capacity(count);
    for _ in 0..count {
        params.push(r.s15fixed16()?);
    }
    Ok(ParametricCurve {
        function_type,
        params,
    })
}

/// Reads a complete embedded curve element (`curv` or `para`) from `r` and advances past its
/// 4-byte-aligned end — the form curves take inside the LUT transform types.
pub(crate) fn read_curve_element(r: &mut ByteReader<'_>) -> Result<CurveOrParametric> {
    let type_sig = r.signature()?;
    r.skip(4)?; // reserved
    let curve = match &type_sig.0 {
        b"curv" => CurveOrParametric::Curve(read_curve_body(r)?),
        b"para" => CurveOrParametric::Parametric(read_parametric_body(r)?),
        _ => {
            return Err(Error::InvalidInput(
                "icc: expected a curv/para curve element",
            ));
        }
    };
    r.align_to_4()?;
    Ok(curve)
}

/// Writes a `curveType` body (count and entries) — the inverse of [`read_curve_body`].
///
/// Rejects a [`Curve::Sampled`] table with fewer than two entries: the on-disk count field is what
/// selects the encoding, so a shorter table would silently re-decode as the identity or a gamma.
pub(crate) fn write_curve_body(curve: &Curve, out: &mut Vec<u8>) -> Result<()> {
    match curve {
        Curve::Identity => out.extend_from_slice(&0u32.to_be_bytes()),
        Curve::Gamma(gamma) => {
            out.extend_from_slice(&1u32.to_be_bytes());
            out.extend_from_slice(&gamma.0.to_be_bytes());
        }
        Curve::Sampled(table) => {
            if table.len() < 2 {
                return Err(Error::InvalidInput(
                    "icc: sampled curve needs at least two entries",
                ));
            }
            out.extend_from_slice(&(table.len() as u32).to_be_bytes());
            for &entry in table {
                out.extend_from_slice(&entry.to_be_bytes());
            }
        }
    }
    Ok(())
}

/// Writes a `parametricCurveType` body (function type and parameters).
///
/// Rejects a parameter count that does not match the function type (the decoder derives the count
/// from the type, so a mismatch would re-decode as different parameters).
pub(crate) fn write_parametric_body(curve: &ParametricCurve, out: &mut Vec<u8>) -> Result<()> {
    if curve.params.len() != parametric_param_count(curve.function_type)? {
        return Err(Error::InvalidInput(
            "icc: parametric parameter count does not match the function type",
        ));
    }
    out.extend_from_slice(&curve.function_type.to_be_bytes());
    out.extend_from_slice(&[0, 0]); // reserved
    for &param in &curve.params {
        push_s15fixed16(out, param);
    }
    Ok(())
}

/// Writes a complete embedded curve element (`curv`/`para`), 4-byte aligned, as it appears inside a
/// LUT transform — the inverse of [`read_curve_element`].
pub(crate) fn write_curve_element(curve: &CurveOrParametric, out: &mut Vec<u8>) -> Result<()> {
    match curve {
        CurveOrParametric::Curve(curve) => {
            out.extend_from_slice(b"curv");
            out.extend_from_slice(&[0; 4]);
            write_curve_body(curve, out)?;
        }
        CurveOrParametric::Parametric(curve) => {
            out.extend_from_slice(b"para");
            out.extend_from_slice(&[0; 4]);
            write_parametric_body(curve, out)?;
        }
    }
    pad_to_4(out);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s15(v: f64) -> S15Fixed16 {
        S15Fixed16::from_f64(v)
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1.0e-6
    }

    #[test]
    fn curve_identity_and_gamma() {
        assert_eq!(Curve::Identity.eval(0.42), 0.42);
        // u8Fixed8 0x0200 == 2.0, so eval(0.5) == 0.25.
        assert!(close(Curve::Gamma(U8Fixed8(0x0200)).eval(0.5), 0.25));
    }

    #[test]
    fn curve_sampled_interpolates() {
        let c = Curve::Sampled(vec![0, 32768, 65535]);
        assert!(close(c.eval(0.0), 0.0));
        assert!(close(c.eval(1.0), 1.0));
        // Halfway to the first node: between 0 and 32768 at fraction 0.5.
        assert!(close(c.eval(0.25), 16384.0 / 65535.0));
        // Halfway to the second node (lower index 1, both endpoints non-zero) — pins the
        // interpolation offset and weighting.
        assert!(close(c.eval(0.75), (32768.0 + 16383.5) / 65535.0));
        // Degenerate tables still evaluate.
        assert!(close(
            Curve::Sampled(vec![32768]).eval(0.7),
            32768.0 / 65535.0
        ));
        assert_eq!(Curve::Sampled(vec![]).eval(0.3), 0.3);
    }

    #[test]
    fn curve_or_parametric_evaluates_both_arms() {
        assert!(close(
            CurveOrParametric::Curve(Curve::Gamma(U8Fixed8(0x0200))).eval(0.5),
            0.25
        ));
        assert!(close(
            CurveOrParametric::Parametric(ParametricCurve {
                function_type: 0,
                params: vec![s15(2.0)],
            })
            .eval(0.5),
            0.25
        ));
    }

    #[test]
    fn read_curve_body_ignores_trailing_bytes() {
        // A sampled curve (count 2) followed by extra bytes still decodes — the bound requires
        // *enough* bytes, not an exact length.
        let mut buf = 2u32.to_be_bytes().to_vec();
        buf.extend_from_slice(&100u16.to_be_bytes());
        buf.extend_from_slice(&200u16.to_be_bytes());
        buf.extend_from_slice(&[0xAB, 0xCD, 0xEF, 0x12]); // trailing padding
        let curve = read_curve_body(&mut ByteReader::new(&buf)).unwrap();
        assert_eq!(curve, Curve::Sampled(vec![100, 200]));
    }

    #[test]
    fn parametric_type0_is_gamma() {
        let curve = ParametricCurve {
            function_type: 0,
            params: vec![s15(2.0)],
        };
        assert!(close(curve.eval(0.5), 0.25));
    }

    #[test]
    fn parametric_type1_clips_below_threshold() {
        // g=1, a=1, b=-0.5 → threshold X >= 0.5.
        let curve = ParametricCurve {
            function_type: 1,
            params: vec![s15(1.0), s15(1.0), s15(-0.5)],
        };
        assert!(close(curve.eval(0.75), 0.25)); // (0.75 - 0.5)
        assert!(close(curve.eval(0.25), 0.0)); // below threshold
    }

    #[test]
    fn parametric_type2_offsets_by_c() {
        // g=1, a=1, b=-0.5, c=0.25 (all exactly representable in s15Fixed16).
        let curve = ParametricCurve {
            function_type: 2,
            params: vec![s15(1.0), s15(1.0), s15(-0.5), s15(0.25)],
        };
        assert!(close(curve.eval(0.75), 0.5)); // (0.75 - 0.5) + 0.25
        assert!(close(curve.eval(0.25), 0.25)); // c
    }

    #[test]
    fn parametric_type3_has_linear_toe() {
        // g=1, a=1, b=0.0625, c=0.5, d=0.5 (a non-zero b pins the `a·x + b` term).
        let curve = ParametricCurve {
            function_type: 3,
            params: vec![s15(1.0), s15(1.0), s15(0.0625), s15(0.5), s15(0.5)],
        };
        assert!(close(curve.eval(0.75), 0.8125)); // (a·x + b)^g
        assert!(close(curve.eval(0.25), 0.125)); // c·x
        assert!(close(curve.eval(0.5), 0.5625)); // at the threshold x == d, the power segment applies
    }

    #[test]
    fn write_rejects_sampled_table_shorter_than_two() {
        // Counts 0 and 1 select the identity/gamma encodings on disk, so writing them from a
        // Sampled table would corrupt the model; two entries is the smallest real table.
        let mut out = Vec::new();
        assert!(write_curve_body(&Curve::Sampled(vec![]), &mut out).is_err());
        assert!(write_curve_body(&Curve::Sampled(vec![100]), &mut out).is_err());
        assert!(write_curve_body(&Curve::Sampled(vec![100, 200]), &mut out).is_ok());
    }

    #[test]
    fn write_rejects_parametric_param_count_mismatch() {
        let mut out = Vec::new();
        // Type 0 takes exactly one parameter.
        let too_many = ParametricCurve {
            function_type: 0,
            params: vec![s15(2.0), s15(1.0)],
        };
        assert!(write_parametric_body(&too_many, &mut out).is_err());
        // An undefined function type is rejected outright.
        let unknown = ParametricCurve {
            function_type: 9,
            params: vec![s15(2.0)],
        };
        assert!(write_parametric_body(&unknown, &mut out).is_err());
        let valid = ParametricCurve {
            function_type: 0,
            params: vec![s15(2.0)],
        };
        assert!(write_parametric_body(&valid, &mut out).is_ok());
    }

    #[test]
    fn parametric_unknown_function_type_is_identity() {
        let curve = ParametricCurve {
            function_type: 99,
            params: Vec::new(),
        };
        assert_eq!(curve.eval(0.42), 0.42);
    }

    #[test]
    fn parametric_type4_has_offset_linear_toe() {
        // g=1, a=1, b=0.0625, c=0.5, d=0.5, e=0.125, f=0.25 (a non-zero b pins `a·x + b`).
        let curve = ParametricCurve {
            function_type: 4,
            params: vec![
                s15(1.0),
                s15(1.0),
                s15(0.0625),
                s15(0.5),
                s15(0.5),
                s15(0.125),
                s15(0.25),
            ],
        };
        assert!(close(curve.eval(0.75), 0.9375)); // (a·x + b)^g + e
        assert!(close(curve.eval(0.25), 0.375)); // c·x + f
        assert!(close(curve.eval(0.5), 0.6875)); // at x == d, the power+e segment applies
    }
}
