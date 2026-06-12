//! Tiny fixed-size linear-algebra helpers shared by the color-science modules.

/// Multiply a 3×3 matrix `m` by a 3-vector `v`, returning `m · v`.
#[must_use]
pub(crate) fn matvec3(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// Multiply two 3×3 matrices, returning `a · b`.
#[must_use]
pub(crate) fn mat_mul3(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
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
#[must_use]
pub(crate) fn mat_inv_3x3(m: &[[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
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

    #[test]
    fn matvec3_identity() {
        let i = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        assert_eq!(matvec3(&i, [3.0, 4.0, 5.0]), [3.0, 4.0, 5.0]);
    }

    #[test]
    fn mat_mul3_identity() {
        let i = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let m = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 10.0]];
        assert_eq!(mat_mul3(&i, &m), m);
    }

    #[test]
    fn mat_inv_3x3_roundtrip() {
        let m = [[1.0, 2.0, 3.0], [0.0, 1.0, 4.0], [5.0, 6.0, 0.0]];
        let inv = mat_inv_3x3(&m).expect("non-singular");
        let prod = mat_mul3(&m, &inv);
        for (i, row) in prod.iter().enumerate() {
            for (j, &v) in row.iter().enumerate() {
                let want = if i == j { 1.0 } else { 0.0 };
                assert!((v - want).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn mat_inv_3x3_singular_is_none() {
        // Two identical rows ⇒ determinant 0.
        let m = [[1.0, 2.0, 3.0], [1.0, 2.0, 3.0], [4.0, 5.0, 6.0]];
        assert!(mat_inv_3x3(&m).is_none());
    }
}
