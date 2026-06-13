//! Lossless colour-type / bit-depth reductions (the PNG-side space optimisation).
//!
//! Before encoding, an image is scanned for redundancy that a smaller PNG encoding can drop without
//! changing any pixel: an all-opaque alpha channel, identical R=G=B channels, or a palette of ≤256
//! distinct colours. The smallest *estimated* encoding (by raw byte count) is chosen; the actual
//! DEFLATE pass then compresses it. Every reduction is exactly reversible, so the decoded pixels are
//! unchanged — the libpng oracle verifies this.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

/// A chosen reduced encoding for an RGB/RGBA image.
pub(crate) enum Reduced {
    /// 8-bit greyscale (R=G=B, fully opaque).
    Gray8(Vec<u8>),
    /// 8-bit greyscale + alpha (R=G=B with transparency).
    GrayAlpha8(Vec<u8>),
    /// 8-bit RGB (alpha was fully opaque and dropped).
    Rgb8(Vec<u8>),
    /// Indexed colour with the smallest sufficient bit depth.
    Indexed {
        /// Index bit depth (1, 2, 4, or 8).
        depth: u8,
        /// One palette index per pixel (one byte each, pre-packing).
        indices: Vec<u8>,
        /// PLTE payload (RGB triples).
        plte: Vec<u8>,
        /// tRNS payload (palette alphas), if the palette is not fully opaque.
        trns: Option<Vec<u8>>,
    },
}

/// The smallest indexed bit depth (1, 2, 4, or 8) that can address `palette_len` entries.
pub(crate) fn index_bit_depth(palette_len: usize) -> u8 {
    match palette_len {
        0..=2 => 1,
        3..=4 => 2,
        5..=16 => 4,
        _ => 8,
    }
}

/// Analyses interleaved 8-bit pixels (`channels` is 3 for RGB or 4 for RGBA) and returns the
/// smallest lossless reduction that beats the input encoding, or `None` to keep it as-is.
pub(crate) fn analyze(pixels: &[u8], channels: usize) -> Option<Reduced> {
    debug_assert!(channels == 3 || channels == 4);
    let pixel_count = pixels.len() / channels;

    let mut all_opaque = true;
    let mut all_gray = true;
    let mut palette_index: HashMap<[u8; 4], u8> = HashMap::new();
    let mut palette: Vec<[u8; 4]> = Vec::new();
    let mut too_many_colors = false;
    for px in pixels.chunks_exact(channels) {
        let alpha = if channels == 4 { px[3] } else { 255 };
        all_opaque &= alpha == 255;
        all_gray &= px[0] == px[1] && px[1] == px[2];
        if !too_many_colors {
            let key = [px[0], px[1], px[2], alpha];
            if let Entry::Vacant(slot) = palette_index.entry(key) {
                if palette.len() == 256 {
                    too_many_colors = true;
                } else {
                    slot.insert(palette.len() as u8);
                    palette.push(key);
                }
            }
        }
    }

    // Estimate the raw size (bytes before compression) of each viable encoding; smaller is better.
    let input_size = pixel_count * channels;
    let palette_size = if too_many_colors {
        usize::MAX
    } else {
        let depth = index_bit_depth(palette.len());
        let needs_trns = palette.iter().any(|c| c[3] != 255);
        let overhead = palette.len() * 3 + if needs_trns { palette.len() } else { 0 } + 24;
        pixel_count * depth as usize / 8 + overhead
    };
    let gray_size = if all_gray && all_opaque {
        pixel_count
    } else {
        usize::MAX
    };
    let gray_alpha_size = if all_gray && !all_opaque {
        pixel_count * 2
    } else {
        usize::MAX
    };
    let rgb_size = if channels == 4 && all_opaque && !all_gray {
        pixel_count * 3
    } else {
        usize::MAX
    };

    let best = palette_size
        .min(gray_size)
        .min(gray_alpha_size)
        .min(rgb_size);
    if best >= input_size {
        return None; // no reduction is smaller
    }

    if best == gray_size {
        Some(Reduced::Gray8(
            pixels.chunks_exact(channels).map(|px| px[0]).collect(),
        ))
    } else if best == gray_alpha_size {
        let mut out = Vec::with_capacity(pixel_count * 2);
        for px in pixels.chunks_exact(channels) {
            out.push(px[0]);
            out.push(px[3]);
        }
        Some(Reduced::GrayAlpha8(out))
    } else if best == rgb_size {
        let mut out = Vec::with_capacity(pixel_count * 3);
        for px in pixels.chunks_exact(channels) {
            out.extend_from_slice(&px[0..3]);
        }
        Some(Reduced::Rgb8(out))
    } else {
        Some(build_indexed(pixels, channels, &palette, &palette_index))
    }
}

/// Builds the indexed reduction from the collected palette.
fn build_indexed(
    pixels: &[u8],
    channels: usize,
    palette: &[[u8; 4]],
    palette_index: &HashMap<[u8; 4], u8>,
) -> Reduced {
    let indices: Vec<u8> = pixels
        .chunks_exact(channels)
        .map(|px| {
            let alpha = if channels == 4 { px[3] } else { 255 };
            *palette_index
                .get(&[px[0], px[1], px[2], alpha])
                .unwrap_or(&0)
        })
        .collect();
    let plte: Vec<u8> = palette.iter().flat_map(|c| [c[0], c[1], c[2]]).collect();
    let trns = if palette.iter().any(|c| c[3] != 255) {
        let mut alphas: Vec<u8> = palette.iter().map(|c| c[3]).collect();
        // Trailing fully-opaque entries may be omitted (they default to opaque).
        while alphas.len() > 1 && alphas.last() == Some(&255) {
            alphas.pop();
        }
        Some(alphas)
    } else {
        None
    };
    Reduced::Indexed {
        depth: index_bit_depth(palette.len()),
        indices,
        plte,
        trns,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_opaque_alpha() {
        // Opaque, non-grey RGBA -> RGB.
        let rgba = [10, 20, 30, 255, 40, 50, 60, 255];
        match analyze(&rgba, 4) {
            Some(Reduced::Rgb8(rgb)) => assert_eq!(rgb, vec![10, 20, 30, 40, 50, 60]),
            _ => panic!("expected Rgb8"),
        }
    }

    #[test]
    fn detects_grayscale() {
        // Opaque R=G=B RGB with many levels -> Gray8.
        let rgb: Vec<u8> = (0..60u8).flat_map(|v| [v, v, v]).collect();
        match analyze(&rgb, 3) {
            Some(Reduced::Gray8(g)) => assert_eq!(g, (0..60u8).collect::<Vec<_>>()),
            _ => panic!("expected Gray8"),
        }
    }

    #[test]
    fn builds_palette_for_few_colours() {
        // Two distinct colours over many pixels -> indexed at 1 bit.
        let mut rgb = Vec::new();
        for i in 0..100u32 {
            if i % 2 == 0 {
                rgb.extend_from_slice(&[200, 10, 10]);
            } else {
                rgb.extend_from_slice(&[10, 10, 200]);
            }
        }
        match analyze(&rgb, 3) {
            Some(Reduced::Indexed {
                depth,
                plte,
                trns,
                indices,
            }) => {
                assert_eq!(depth, 1);
                assert_eq!(plte.len(), 6); // two RGB entries
                assert!(trns.is_none());
                assert_eq!(indices.len(), 100);
            }
            _ => panic!("expected Indexed"),
        }
    }

    #[test]
    fn keeps_full_colour_photographic_data() {
        // Many distinct opaque colours, not grey -> no reduction.
        let rgb: Vec<u8> = (0..300u32)
            .flat_map(|i| [i as u8, (i >> 1) as u8, (i >> 2) as u8])
            .collect();
        assert!(analyze(&rgb, 3).is_none());
    }

    #[test]
    fn palette_with_transparency_emits_trns() {
        let rgba = [
            0, 0, 0, 0, // transparent black
            255, 255, 255, 255, // opaque white
        ]
        .repeat(20);
        match analyze(&rgba, 4) {
            Some(Reduced::Indexed { trns: Some(t), .. }) => assert_eq!(t, vec![0]),
            _ => panic!("expected indexed with tRNS"),
        }
    }
}
