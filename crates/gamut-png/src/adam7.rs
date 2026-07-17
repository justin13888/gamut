//! Adam7 interlacing geometry (PNG spec §8.1) and de-interlace recomposition (§13.10).
//!
//! An interlaced image is transmitted as seven reduced images, each a sub-lattice of the full
//! pixel grid described by an origin and a stride. Reduced images with zero width or height are
//! entirely absent from the stream — including their filter-type bytes (§7.3). Each reduced image
//! is filtered and byte-padded independently (§7.2), so the decoder defilters and unpacks per
//! pass, then scatters the samples back onto the full-size canvas.

use crate::ihdr::Ihdr;

/// One pass over the pixel lattice: the origin and stride of its reduced image.
pub(crate) struct Pass {
    /// X of the first pixel the pass covers.
    pub x0: u32,
    /// Y of the first pixel the pass covers.
    pub y0: u32,
    /// Horizontal stride between covered pixels.
    pub dx: u32,
    /// Vertical stride between covered pixels.
    pub dy: u32,
}

/// The seven Adam7 passes, in transmission order (§8.1).
pub(crate) const PASSES: [Pass; 7] = [
    Pass {
        x0: 0,
        y0: 0,
        dx: 8,
        dy: 8,
    },
    Pass {
        x0: 4,
        y0: 0,
        dx: 8,
        dy: 8,
    },
    Pass {
        x0: 0,
        y0: 4,
        dx: 4,
        dy: 8,
    },
    Pass {
        x0: 2,
        y0: 0,
        dx: 4,
        dy: 4,
    },
    Pass {
        x0: 0,
        y0: 2,
        dx: 2,
        dy: 4,
    },
    Pass {
        x0: 1,
        y0: 0,
        dx: 2,
        dy: 2,
    },
    Pass {
        x0: 0,
        y0: 1,
        dx: 1,
        dy: 2,
    },
];

/// The trivial single "pass" of a non-interlaced image, so both modes share one pipeline.
pub(crate) const SEQUENTIAL: [Pass; 1] = [Pass {
    x0: 0,
    y0: 0,
    dx: 1,
    dy: 1,
}];

/// The pass list for an image's interlace mode.
pub(crate) fn passes_for(interlaced: bool) -> &'static [Pass] {
    if interlaced { &PASSES } else { &SEQUENTIAL }
}

/// The reduced image's dimensions for one pass: `ceil((dim − origin) / stride)`, which is zero
/// when the image is too small to reach the pass's origin (an empty pass, §13.10).
pub(crate) fn pass_dimensions(pass: &Pass, width: u32, height: u32) -> (u32, u32) {
    (
        width.saturating_sub(pass.x0).div_ceil(pass.dx),
        height.saturating_sub(pass.y0).div_ceil(pass.dy),
    )
}

/// The exact byte length of the whole filtered scanline stream: for every non-empty reduced
/// image, each scanline is one filter-type byte plus its packed row (§7.2/§7.3); empty passes
/// contribute nothing. `None` on arithmetic overflow.
pub(crate) fn expected_stream_len(header: &Ihdr) -> Option<usize> {
    let mut total = 0usize;
    for pass in passes_for(header.interlaced) {
        let (pass_width, pass_height) = pass_dimensions(pass, header.width, header.height);
        if pass_width == 0 || pass_height == 0 {
            continue;
        }
        let row_bytes = (pass_width as usize)
            .checked_mul(header.bits_per_pixel())?
            .div_ceil(8);
        let pass_bytes = (pass_height as usize).checked_mul(row_bytes.checked_add(1)?)?;
        total = total.checked_add(pass_bytes)?;
    }
    Some(total)
}

/// Scatters one pass's samples onto the full-size canvas: pass pixel `(i, j)` lands at
/// `(x0 + i·dx, y0 + j·dy)` (§8.1). Generic over the sample type so 8- and 16-bit decoding share
/// the implementation. `pass_samples` holds `pass_width × pass_height × channels` samples.
pub(crate) fn scatter<S: Copy>(
    canvas: &mut [S],
    canvas_width: usize,
    pass: &Pass,
    pass_samples: &[S],
    pass_width: usize,
    channels: usize,
) {
    debug_assert!(pass_width > 0);
    let pass_height = pass_samples.len() / (pass_width * channels);
    for j in 0..pass_height {
        let y = pass.y0 as usize + j * pass.dy as usize;
        for i in 0..pass_width {
            let x = pass.x0 as usize + i * pass.dx as usize;
            let src = (j * pass_width + i) * channels;
            let dst = (y * canvas_width + x) * channels;
            canvas[dst..dst + channels].copy_from_slice(&pass_samples[src..src + channels]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::ColorType;

    fn header(width: u32, height: u32, bit_depth: u8, interlaced: bool) -> Ihdr {
        Ihdr {
            width,
            height,
            bit_depth,
            color: ColorType::Grayscale,
            interlaced,
        }
    }

    #[test]
    fn eight_by_eight_pass_dimensions_match_the_spec_grid() {
        let expected = [(1, 1), (1, 1), (2, 1), (2, 2), (4, 2), (4, 4), (8, 4)];
        for (pass, want) in PASSES.iter().zip(expected) {
            assert_eq!(pass_dimensions(pass, 8, 8), want);
        }
        // The seven reduced images cover each pixel exactly once.
        let total: u32 = PASSES
            .iter()
            .map(|p| {
                let (w, h) = pass_dimensions(p, 8, 8);
                w * h
            })
            .sum();
        assert_eq!(total, 64);
    }

    #[test]
    fn small_images_empty_the_right_passes() {
        // 1×1: only pass 1 has a pixel.
        let non_empty: Vec<usize> = PASSES
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                let (w, h) = pass_dimensions(p, 1, 1);
                w > 0 && h > 0
            })
            .map(|(i, _)| i + 1)
            .collect();
        assert_eq!(non_empty, vec![1]);
        // 3×3 per the worked example in the decoder tests.
        let dims: Vec<(u32, u32)> = PASSES.iter().map(|p| pass_dimensions(p, 3, 3)).collect();
        assert_eq!(
            dims,
            vec![(1, 1), (0, 1), (1, 0), (1, 1), (2, 1), (1, 2), (3, 1)]
        );
        // Every size decomposes without loss or overlap.
        for w in 1..=20u32 {
            for h in 1..=20u32 {
                let total: u32 = PASSES
                    .iter()
                    .map(|p| {
                        let (pw, ph) = pass_dimensions(p, w, h);
                        pw * ph
                    })
                    .sum();
                assert_eq!(total, w * h, "{w}x{h}");
            }
        }
    }

    #[test]
    fn stream_length_counts_filter_bytes_per_non_empty_scanline() {
        // Non-interlaced 5×3 grey8: 3 rows of (1 + 5) bytes.
        assert_eq!(expected_stream_len(&header(5, 3, 8, false)), Some(18));
        // Interlaced 1×1: a single 1-pixel pass, 1 filter byte + 1 sample.
        assert_eq!(expected_stream_len(&header(1, 1, 8, true)), Some(2));
        // Interlaced 8×8 grey8: sum over the spec grid, one filter byte per scanline.
        let expected: usize = [(1u32, 1u32), (1, 1), (2, 1), (2, 2), (4, 2), (4, 4), (8, 4)]
            .iter()
            .map(|&(w, h)| h as usize * (1 + w as usize))
            .sum();
        assert_eq!(expected_stream_len(&header(8, 8, 8, true)), Some(expected));
        // Sub-byte rows are padded per pass: interlaced 3×3 at 1-bit each row is 1 packed byte.
        assert_eq!(expected_stream_len(&header(3, 3, 1, true)), Some(12));
    }

    #[test]
    fn scatter_places_pass_pixels_on_the_lattice() {
        // Pass 6 of a 4×2 image (x0=1, dx=2, y0=0, dy=2): pixels land at x = 1, 3 on row 0.
        let mut canvas = [0u8; 8];
        let pass = &PASSES[5];
        let (pw, ph) = pass_dimensions(pass, 4, 2);
        assert_eq!((pw, ph), (2, 1));
        scatter(&mut canvas, 4, pass, &[7, 9], 2, 1);
        assert_eq!(canvas, [0, 7, 0, 9, 0, 0, 0, 0]);
        // Two-channel scatter keeps channel adjacency.
        let mut canvas2 = [0u16; 8];
        scatter(
            &mut canvas2,
            2,
            &SEQUENTIAL[0],
            &[1, 2, 3, 4, 5, 6, 7, 8],
            2,
            2,
        );
        assert_eq!(canvas2, [1, 2, 3, 4, 5, 6, 7, 8]);
    }
}
