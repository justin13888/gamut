//! The WebP `ALPH` alpha chunk (RFC 9649 §2.7.1): an 8-bit transparency plane stored alongside a
//! lossy `VP8 ` bitstream in an extended (`VP8X`) file.
//!
//! The plane is optionally **filtered** — each value predicted from its left, top, or
//! left+top−topleft (gradient) neighbor, the residual stored — which decorrelates it so the
//! lossless compressor (method `1`, [`crate::vp8l`]) packs it tighter. The filter is bijective and
//! lossless regardless of how the residuals are stored, so raw storage (method `0`) round-trips the
//! alpha exactly. This module implements the chunk header, the four filters, and **raw** storage;
//! lossless-compressed storage layers on top in a later milestone.

use gamut_core::{Error, Result};

/// Alpha filtering method (RFC 9649 §2.7.1 Figure 10, field `F`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaFilter {
    /// No filtering: the predictor is always 0, so residuals are the alpha values themselves.
    None,
    /// Horizontal: predict each value from the pixel to its left.
    Horizontal,
    /// Vertical: predict each value from the pixel above.
    Vertical,
    /// Gradient: predict from `clip(left + top − topleft)`.
    Gradient,
}

impl AlphaFilter {
    /// The 2-bit `F` field value.
    fn code(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Horizontal => 1,
            Self::Vertical => 2,
            Self::Gradient => 3,
        }
    }

    /// Decodes the 2-bit `F` field.
    fn from_code(code: u8) -> Self {
        match code & 0x3 {
            1 => Self::Horizontal,
            2 => Self::Vertical,
            3 => Self::Gradient,
            _ => Self::None,
        }
    }

    /// The predictor for pixel `(x, y)` of a `width`-wide alpha `plane`, reading only already-known
    /// pixels (RFC 9649 §2.7.1). The top-left pixel predicts 0; the leftmost column of the horizontal
    /// and gradient filters predicts from the pixel above; the top row of the vertical and gradient
    /// filters predicts from the pixel to the left.
    fn predict(self, plane: &[u8], width: usize, x: usize, y: usize) -> u8 {
        let at = |x: usize, y: usize| plane[y * width + x];
        if x == 0 && y == 0 {
            return 0;
        }
        match self {
            Self::None => 0,
            Self::Horizontal => {
                if x > 0 {
                    at(x - 1, y)
                } else {
                    at(0, y - 1)
                }
            }
            Self::Vertical => {
                if y > 0 {
                    at(x, y - 1)
                } else {
                    at(x - 1, 0)
                }
            }
            Self::Gradient => {
                if x == 0 {
                    at(0, y - 1)
                } else if y == 0 {
                    at(x - 1, 0)
                } else {
                    let (a, b, c) = (
                        i32::from(at(x - 1, y)),
                        i32::from(at(x, y - 1)),
                        i32::from(at(x - 1, y - 1)),
                    );
                    (a + b - c).clamp(0, 255) as u8
                }
            }
        }
    }
}

/// Filters an alpha `plane` (`width * height` bytes, scan order), returning the residuals
/// `(alpha − predictor) mod 256` (RFC 9649 §2.7.1). The predictor reads the original plane, which —
/// because the transform is lossless — equals the values the decoder will have reconstructed.
#[must_use]
pub fn filter(plane: &[u8], width: usize, height: usize, method: AlphaFilter) -> Vec<u8> {
    let mut out = vec![0u8; plane.len()];
    for y in 0..height {
        for x in 0..width {
            let i = y * width + x;
            out[i] = plane[i].wrapping_sub(method.predict(plane, width, x, y));
        }
    }
    out
}

/// Inverts [`filter`]: reconstructs the alpha plane from `residuals`, predicting from the
/// already-reconstructed pixels (`alpha = (predictor + residual) mod 256`).
#[must_use]
pub fn unfilter(residuals: &[u8], width: usize, height: usize, method: AlphaFilter) -> Vec<u8> {
    let mut plane = vec![0u8; residuals.len()];
    for y in 0..height {
        for x in 0..width {
            let i = y * width + x;
            let pred = method.predict(&plane, width, x, y);
            plane[i] = pred.wrapping_add(residuals[i]);
        }
    }
    plane
}

/// Picks the alpha filter whose residuals have the least total magnitude — a cheap proxy for "most
/// compressible" (the choice only affects size, never correctness). Treating residuals as signed
/// (distance from 0 or 256) favors the smoothest predictor.
#[must_use]
pub fn choose_filter(plane: &[u8], width: usize, height: usize) -> AlphaFilter {
    [
        AlphaFilter::None,
        AlphaFilter::Horizontal,
        AlphaFilter::Vertical,
        AlphaFilter::Gradient,
    ]
    .into_iter()
    .min_by_key(|&m| {
        filter(plane, width, height, m)
            .iter()
            .map(|&r| u32::from(r.min(r.wrapping_neg())))
            .sum::<u32>()
    })
    .unwrap_or(AlphaFilter::None)
}

/// Builds an `ALPH` chunk payload (RFC 9649 §2.7.1) for a **raw** (uncompressed) alpha plane: the
/// header byte (`Rsv=0`, `P=0`, the filter method, `C=0`) followed by the filtered residuals.
#[must_use]
pub fn write_raw_alph(plane: &[u8], width: usize, height: usize) -> Vec<u8> {
    let method = choose_filter(plane, width, height);
    let mut out = Vec::with_capacity(1 + plane.len());
    out.push(method.code() << 2); // P = 0, C = 0 (no compression)
    out.extend_from_slice(&filter(plane, width, height, method));
    out
}

/// Decodes an `ALPH` chunk payload into the alpha plane (RFC 9649 §2.7.1). Only the raw compression
/// method (`C = 0`) is handled here; lossless-compressed alpha (`C = 1`) is a later milestone.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] for an empty payload or a raw payload of the wrong length, and
/// [`Error::Unsupported`] for the lossless compression method.
pub fn read_alph(payload: &[u8], width: usize, height: usize) -> Result<Vec<u8>> {
    let &header = payload
        .first()
        .ok_or(Error::InvalidInput("ALPH: empty chunk"))?;
    let method = AlphaFilter::from_code(header >> 2);
    let compression = header & 0x3;
    if compression != 0 {
        return Err(Error::Unsupported(
            "ALPH: lossless-compressed alpha not yet supported",
        ));
    }
    let residuals = &payload[1..];
    if residuals.len() != width * height {
        return Err(Error::InvalidInput("ALPH: raw alpha length mismatch"));
    }
    Ok(unfilter(residuals, width, height, method))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern(width: usize, height: usize) -> Vec<u8> {
        (0..width * height)
            .map(|i| {
                let (x, y) = (i % width, i / width);
                ((x * 9 + y * 5 + (x ^ y) * 3) & 0xff) as u8
            })
            .collect()
    }

    #[test]
    fn each_filter_inverts_exactly() {
        let (w, h) = (23, 17);
        let plane = pattern(w, h);
        for m in [
            AlphaFilter::None,
            AlphaFilter::Horizontal,
            AlphaFilter::Vertical,
            AlphaFilter::Gradient,
        ] {
            let residuals = filter(&plane, w, h, m);
            assert_eq!(
                unfilter(&residuals, w, h, m),
                plane,
                "filter {m:?} round-trip"
            );
        }
    }

    #[test]
    fn none_filter_stores_alpha_verbatim() {
        let plane = pattern(8, 8);
        assert_eq!(filter(&plane, 8, 8, AlphaFilter::None), plane);
    }

    #[test]
    fn raw_alph_chunk_round_trips() {
        let (w, h) = (19, 11);
        let plane = pattern(w, h);
        let chunk = write_raw_alph(&plane, w, h);
        assert_eq!(chunk.len(), 1 + w * h);
        assert_eq!(chunk[0] & 0x3, 0, "compression method is raw");
        assert_eq!(read_alph(&chunk, w, h).unwrap(), plane);
    }

    #[test]
    fn read_alph_rejects_bad_input() {
        assert!(read_alph(&[], 4, 4).is_err());
        assert!(read_alph(&[0, 1, 2], 4, 4).is_err(), "wrong raw length");
        assert!(read_alph(&[0x01], 0, 0).is_err(), "C=1 unsupported");
    }
}
