//! Tiny fixed-size linear-algebra helpers shared by the colour-science modules.
//!
//! Public (issue #321): the colour-management layer building on this crate (gamut-cmm,
//! issues #323/#327) chains the same 3×3 primitives — profile matrix/TRC transforms are
//! `matvec3` pipelines over matrices composed with [`mat_mul3`] and inverted with
//! [`mat_inv_3x3`] — so the helpers are exported rather than re-implemented downstream.
//! The surface stays deliberately minimal: exactly the 3×3 operations colour pipelines
//! need, not a general linear-algebra crate.

/// Multiply a 3×3 matrix `m` by a 3-vector `v`, returning `m · v`.
///
/// # Examples
///
/// ```
/// use gamut_color::linalg::matvec3;
/// let m = [[1.0, 2.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 3.0]];
/// assert_eq!(matvec3(&m, [1.0, 1.0, 1.0]), [3.0, 1.0, 3.0]);
/// ```
#[must_use]
pub fn matvec3(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// Multiply two 3×3 matrices, returning `a · b`.
///
/// # Examples
///
/// ```
/// use gamut_color::linalg::mat_mul3;
/// let a = [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]; // swap rows 0/1
/// let b = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]];
/// let c = mat_mul3(&a, &b);
/// assert_eq!(c[0], [4.0, 5.0, 6.0]);
/// assert_eq!(c[1], [1.0, 2.0, 3.0]);
/// ```
#[must_use]
pub fn mat_mul3(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut c = [[0.0; 3]; 3];
    for (i, ci) in c.iter_mut().enumerate() {
        for (j, cij) in ci.iter_mut().enumerate() {
            for k in 0..3 {
                *cij += a[i][k] * b[k][j];
            }
        }
    }
    c
}

/// Invert a 3×3 matrix via the cofactor formula. Returns `None` if the matrix is
/// singular (zero or non-finite determinant).
///
/// # Examples
///
/// ```
/// use gamut_color::linalg::mat_inv_3x3;
/// let inv = mat_inv_3x3(&[[2.0, 0.0, 0.0], [0.0, 4.0, 0.0], [0.0, 0.0, 8.0]]).unwrap();
/// assert_eq!(inv, [[0.5, 0.0, 0.0], [0.0, 0.25, 0.0], [0.0, 0.0, 0.125]]);
/// assert!(mat_inv_3x3(&[[0.0; 3]; 3]).is_none()); // singular
/// ```
#[must_use]
pub fn mat_inv_3x3(m: &[[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let [a, b, c] = m[0];
    let [d, e, f] = m[1];
    let [g, h, k] = m[2];
    let det = a * (e * k - f * h) - b * (d * k - f * g) + c * (d * h - e * g);
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    let inv_det = 1.0 / det;
    Some([
        [
            (e * k - f * h) * inv_det,
            (c * h - b * k) * inv_det,
            (b * f - c * e) * inv_det,
        ],
        [
            (f * g - d * k) * inv_det,
            (a * k - c * g) * inv_det,
            (c * d - a * f) * inv_det,
        ],
        [
            (d * h - e * g) * inv_det,
            (b * g - a * h) * inv_det,
            (a * e - b * d) * inv_det,
        ],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    // The dense-input arithmetic of matvec3 / mat_mul3 / mat_inv_3x3 is pinned by this module's
    // consumers: the Lindbloom oracles and the M1 derivation check in `matrix`, and the chromahash
    // golden vectors in `oklab`, run every product term through published values. The tests below
    // add the two paths those leave open: the singular-determinant guard, and an integer-exact
    // inverse (det = 1, so every cofactor must come out exactly — a swapped or sign-flipped
    // cofactor term cannot hide behind a tolerance).
    #[test]
    fn mat_inv_3x3_singular_is_none() {
        // Two identical rows ⇒ determinant 0.
        let m = [[1.0, 2.0, 3.0], [1.0, 2.0, 3.0], [4.0, 5.0, 6.0]];
        assert!(mat_inv_3x3(&m).is_none());
    }

    /// Textbook unimodular matrix (det = 1) with a known integer inverse: f64 integer
    /// arithmetic is exact, so the cofactor formula must reproduce it bit-for-bit.
    #[test]
    fn mat_inv_3x3_matches_known_integer_inverse() {
        let m = [[1.0, 2.0, 3.0], [0.0, 1.0, 4.0], [5.0, 6.0, 0.0]];
        let want = [[-24.0, 18.0, 5.0], [20.0, -15.0, -4.0], [-5.0, 4.0, 1.0]];
        let inv = mat_inv_3x3(&m).expect("det = 1");
        assert_eq!(inv, want);
        // And it really is the inverse: m · m⁻¹ = I exactly.
        let id = mat_mul3(&m, &inv);
        assert_eq!(id, [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
    }
}
