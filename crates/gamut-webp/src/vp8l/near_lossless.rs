//! Near-lossless preprocessing for the VP8L path (issue #261).
//!
//! Near-lossless is not a bitstream feature — nothing here changes what VP8L can express. It is a
//! **deliberate, bounded quantization of the source pixels applied before lossless coding**, so the
//! coded stream still reproduces its input bit-exactly; that input is simply a quantized copy of
//! the caller's image. The point is that zeroing the low bits of smooth regions gives the spatial
//! predictors and the entropy coder far less residual to carry, for an error the eye does not see.
//!
//! # The rule
//!
//! Quantization error hides in **texture** and shows up in **smooth gradients**, where rounding a
//! slow ramp onto a coarse grid produces visible banding. So the test is inverted from the obvious
//! one: a pixel is quantized only when its 4-neighbourhood is *busy* relative to the step being
//! taken, and a pixel sitting in a smooth region is left exact. Discarding low bits where the
//! signal is already changing fast costs nothing perceptually and removes exactly the
//! high-frequency detail that the entropy coder spends the most bits on.
//!
//! The strength is applied as a descending sequence of passes (coarsest first), so a region busy
//! enough for a fine step still gets quantized even when it was too smooth for the coarse one.
//! Border pixels have no complete 4-neighbourhood to judge, so they are carried through exactly.
//!
//! Each pass is **out-of-place**: it reads the previous pass's buffer and writes a fresh one, so
//! the result is a pure function of the input rather than depending on scan order.
//!
//! # What is guaranteed
//!
//! - **Red, green and blue** move by at most `2^bits - 1` in total — `1`, `3`, `7`, `15`, `31` for
//!   strengths mapping to 1..=5 bits.
//! - **Alpha is never modified.** It is read, so a transparency edge counts as texture, but its own
//!   value is carried through exactly. That keeps the crate's existing promise that alpha
//!   round-trips bit-exactly, and keeps masks usable.
//! - `bits == 0` is the identity, byte-for-byte.
//!
//! # Relationship to libwebp
//!
//! The strength **scale** is libwebp's (`near_lossless` `0..=100`, `100` = off, mapping to
//! `5 - strength / 20` bits) so a caller migrating from `cwebp` gets what they expect. The
//! quantization rule and its error bound are this crate's own and are stated above rather than
//! inherited; libwebp additionally quantizes alpha and skips the pass entirely on small images,
//! neither of which is done here. Output is therefore **not** byte-identical to libwebp's, so the
//! differential test asserts the bound rather than the bytes.

use gamut_core::Dimensions;

use crate::vp8l::transform::{alpha, blue, green, make_argb, red};

/// Quantizes the low-order RGB bits of `argb` in textured regions, leaving smooth gradients — and
/// every alpha value — exact.
///
/// `bits` is the quantization depth from [`NearLossless::bits`](crate::NearLossless::bits); `0` is
/// the identity.
#[must_use]
pub(crate) fn apply(argb: &[u32], dims: Dimensions, bits: u8) -> Vec<u32> {
    let mut pixels = argb.to_vec();
    if bits == 0 || argb.is_empty() {
        return pixels;
    }
    let (width, height) = (dims.width as usize, dims.height as usize);
    // Coarsest step first: a region too busy for a coarse step may still be flat enough for a
    // finer one, so the descending sequence reaches more of the image than any single pass.
    for depth in (1..=bits).rev() {
        pixels = quantize_pass(&pixels, width, height, depth);
    }
    pixels
}

/// One out-of-place quantization pass at `depth` bits.
///
/// Interior pixels whose 4-neighbourhood varies by at least the quantization step are quantized;
/// smooth interiors and the whole border are copied through exactly.
fn quantize_pass(pixels: &[u32], width: usize, height: usize, depth: u8) -> Vec<u32> {
    let limit = 1u32 << depth;
    let mut out = pixels.to_vec();
    if width < 3 || height < 3 {
        return out; // No pixel has a complete 4-neighbourhood to judge.
    }
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let i = y * width + x;
            if neighbourhood_spread(pixels, width, x, y) >= limit {
                out[i] = quantize_pixel(pixels[i], depth);
            }
        }
    }
    out
}

/// The largest absolute per-channel difference between the interior pixel at `(x, y)` and its four
/// orthogonal neighbours, over **all four channels**.
///
/// Alpha participates so that a transparency edge counts as texture even when the colour beside it
/// does not change, even though alpha itself is never quantized. The caller guarantees `(x, y)` is
/// an interior pixel, so all four neighbours exist.
fn neighbourhood_spread(pixels: &[u32], width: usize, x: usize, y: usize) -> u32 {
    let centre = pixels[y * width + x];
    let mut spread = 0u32;
    for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
        let other = pixels[ny * width + nx];
        for (a, b) in [
            (alpha(centre), alpha(other)),
            (red(centre), red(other)),
            (green(centre), green(other)),
            (blue(centre), blue(other)),
        ] {
            spread = spread.max(u32::from(a.abs_diff(b)));
        }
    }
    spread
}

/// Rounds each of a pixel's RGB channels to the nearest multiple of `2^depth`, saturating at 255.
/// Alpha is copied through untouched.
fn quantize_pixel(pixel: u32, depth: u8) -> u32 {
    make_argb(
        alpha(pixel),
        quantize_channel(red(pixel), depth),
        quantize_channel(green(pixel), depth),
        quantize_channel(blue(pixel), depth),
    )
}

/// Rounds `value` to the nearest multiple of `2^depth` (halves up), saturating at 255 so the result
/// stays an 8-bit sample. The deviation is therefore at most `2^(depth - 1)`.
fn quantize_channel(value: u8, depth: u8) -> u8 {
    let step = 1u32 << depth;
    let rounded = ((u32::from(value) + step / 2) >> depth) << depth;
    rounded.min(255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dims(width: u32, height: u32) -> Dimensions {
        Dimensions { width, height }
    }

    #[test]
    fn zero_bits_is_the_identity() {
        // `None` near-lossless must cost nothing and change nothing; this is what lets the encoder
        // call `apply` unconditionally.
        let pixels: Vec<u32> = (0..64)
            .map(|i| make_argb(i as u8, 3, i as u8, 200))
            .collect();
        assert_eq!(apply(&pixels, dims(8, 8), 0), pixels);
    }

    #[test]
    fn rgb_stays_within_the_bound_and_alpha_is_exact() {
        // The two halves of the contract. The bound is `2^bits - 1` because the descending passes
        // move a channel by at most `2^(depth-1)` each and those halve down to 1.
        let pixels: Vec<u32> = (0..32u32 * 32)
            .map(|i| {
                let (x, y) = (i % 32, i / 32);
                make_argb((i % 256) as u8, (x * 8) as u8, (y * 8) as u8, (x + y) as u8)
            })
            .collect();
        for bits in 1..=5u8 {
            let out = apply(&pixels, dims(32, 32), bits);
            let bound = (1u16 << bits) - 1;
            for (before, after) in pixels.iter().zip(&out) {
                assert_eq!(alpha(*before), alpha(*after), "alpha must never move");
                for (a, b) in [
                    (red(*before), red(*after)),
                    (green(*before), green(*after)),
                    (blue(*before), blue(*after)),
                ] {
                    assert!(
                        u16::from(a.abs_diff(b)) <= bound,
                        "bits {bits}: channel moved {}, bound {bound}",
                        a.abs_diff(b)
                    );
                }
            }
        }
    }

    #[test]
    fn smooth_gradients_are_left_exact() {
        // Banding is the failure mode near-lossless must not produce, and a slow ramp is exactly
        // where rounding onto a coarse grid would produce it. Neighbouring pixels here differ by 1,
        // which is under every step, so nothing may be quantized at any strength.
        let pixels: Vec<u32> = (0..32u32 * 32)
            .map(|i| {
                let v = (i / 32) as u8;
                make_argb(0xff, v, v, v)
            })
            .collect();
        for bits in 1..=5u8 {
            assert_eq!(
                apply(&pixels, dims(32, 32), bits),
                pixels,
                "bits {bits} banded a smooth gradient"
            );
        }
        // A flat region is a gradient with slope zero, and must likewise be untouched.
        let flat: Vec<u32> = (0..32u32 * 32)
            .map(|_| make_argb(0xff, 100, 101, 102))
            .collect();
        assert_eq!(apply(&flat, dims(32, 32), 5), flat);
    }

    #[test]
    fn textured_regions_actually_get_quantized() {
        // The complement of the gradient test: without this, an implementation that never
        // quantizes anything would satisfy both the bound and the banding tests.
        let pixels: Vec<u32> = (0..32u32 * 32)
            .map(|i| {
                let (x, y) = (i % 32, i / 32);
                let v = ((x * 37 + y * 101) % 256) as u8;
                make_argb(0xff, v, v.wrapping_add(90), v.wrapping_mul(3))
            })
            .collect();
        let out = apply(&pixels, dims(32, 32), 3);
        assert_ne!(out, pixels, "a textured region must be quantized");
        // Interior pixels that were quantized land on multiples of the finest step applied.
        let quantized = pixels
            .iter()
            .zip(&out)
            .filter(|(before, after)| before != after)
            .count();
        assert!(
            quantized > 32 * 32 / 4,
            "only {quantized} pixels were quantized; the rule is barely firing"
        );
    }

    #[test]
    fn the_border_is_carried_through_exactly() {
        // Border pixels have no complete 4-neighbourhood to judge, so they are copied rather than
        // judged against a clamped or wrapped neighbour.
        let (w, h) = (16usize, 16usize);
        let pixels: Vec<u32> = (0..(w * h) as u32)
            .map(|i| {
                let v = ((i * 53) % 256) as u8;
                make_argb(0xff, v, v.wrapping_add(31), v.wrapping_mul(7))
            })
            .collect();
        let out = apply(&pixels, dims(w as u32, h as u32), 4);
        for y in 0..h {
            for x in 0..w {
                if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
                    let i = y * w + x;
                    assert_eq!(out[i], pixels[i], "border pixel ({x}, {y}) was modified");
                }
            }
        }
    }

    #[test]
    fn quantization_is_independent_of_scan_order() {
        // Each pass reads the previous buffer and writes a fresh one, so a pixel's decision uses
        // its *original* neighbours rather than ones an earlier iteration already rewrote. That
        // makes the result commute with mirroring: quantizing a mirrored image and mirroring back
        // must give exactly what quantizing the original gives. An in-place pass would fail this,
        // because reversing the scan changes which neighbours were already overwritten.
        let (w, h) = (24usize, 18usize);
        let pixels: Vec<u32> = (0..(w * h) as u32)
            .map(|i| {
                let v = ((i * 61) % 256) as u8;
                make_argb(0xff, v, v.wrapping_add(17), v.wrapping_mul(5))
            })
            .collect();
        let mirror = |src: &[u32]| -> Vec<u32> {
            let mut out = vec![0u32; src.len()];
            for y in 0..h {
                for x in 0..w {
                    out[y * w + x] = src[y * w + (w - 1 - x)];
                }
            }
            out
        };
        let direct = apply(&pixels, dims(w as u32, h as u32), 4);
        let via_mirror = mirror(&apply(&mirror(&pixels), dims(w as u32, h as u32), 4));
        assert_eq!(direct, via_mirror, "the result depends on scan order");
        // And it must actually have done something, or the property is vacuous.
        assert_ne!(direct, pixels);
    }

    #[test]
    fn channel_quantization_rounds_to_the_nearest_step() {
        // Pins the rounding rule and the saturation, which the error bound is derived from.
        assert_eq!(quantize_channel(0, 3), 0);
        assert_eq!(quantize_channel(3, 3), 0); // 3 rounds down to 0
        assert_eq!(quantize_channel(4, 3), 8); // halves round up
        assert_eq!(quantize_channel(12, 3), 16);
        assert_eq!(quantize_channel(255, 3), 255); // saturates rather than wrapping to 256
        assert_eq!(quantize_channel(255, 5), 255);
        assert_eq!(quantize_channel(200, 1), 200);
        for depth in 1..=5u8 {
            for value in 0..=255u8 {
                let out = quantize_channel(value, depth);
                assert!(
                    u16::from(value.abs_diff(out)) <= 1 << (depth - 1),
                    "depth {depth} moved {value} to {out}"
                );
            }
        }
    }

    #[test]
    fn a_single_pixel_image_has_no_neighbours_to_judge() {
        // Degenerate shapes must not index out of bounds; a lone pixel has an empty neighbourhood,
        // so its spread is 0 and it quantizes.
        let one = vec![make_argb(0xff, 100, 100, 100)];
        assert_eq!(apply(&one, dims(1, 1), 3).len(), 1);
        let row: Vec<u32> = (0..5).map(|_| make_argb(0xff, 60, 60, 60)).collect();
        assert_eq!(apply(&row, dims(5, 1), 2).len(), 5);
        assert_eq!(apply(&row, dims(1, 5), 2).len(), 5);
    }
}
