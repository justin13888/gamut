//! Lossless colour-type / bit-depth reductions (the PNG-side space optimisation).
//!
//! Before encoding, an image is scanned for redundancy that a smaller PNG encoding can drop without
//! changing any pixel: an all-opaque alpha channel, identical R=G=B channels, a palette of ≤256
//! distinct colours, grey values exactly representable at a sub-byte depth (§13.12), or 16-bit
//! samples whose high and low bytes agree (lossless 16→8 demotion). The smallest *estimated*
//! encoding (by raw byte count; sub-byte row padding is ignored, as in the palette estimate) is
//! chosen; the actual DEFLATE pass then compresses it. Every reduction is exactly reversible, so
//! the decoded pixels are unchanged — the libpng oracle verifies this.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use crate::pack::gray8_scale;

/// A chosen reduced encoding for an image.
pub enum Reduced {
    /// Greyscale at depth 1, 2, 4, or 8 (R=G=B, fully opaque). `samples` holds one byte per pixel:
    /// the raw value at depth 8, the unscaled code (`value / gray8_scale(depth)`) below it.
    Gray {
        /// Grey bit depth (1, 2, 4, or 8).
        depth: u8,
        /// One sample per pixel (one byte each, pre-packing).
        samples: Vec<u8>,
    },
    /// 8-bit greyscale + alpha (R=G=B with transparency).
    GrayAlpha8(Vec<u8>),
    /// 8-bit RGB (alpha was fully opaque and dropped).
    Rgb8(Vec<u8>),
    /// 8-bit RGBA: a 16-bit input demoted losslessly, with no further channel reduction.
    Rgba8(Vec<u8>),
    /// 16-bit greyscale (R=G=B, fully opaque), pre-serialised big-endian.
    Gray16Be(Vec<u8>),
    /// 16-bit greyscale + alpha (R=G=B with transparency), pre-serialised big-endian.
    GrayAlpha16Be(Vec<u8>),
    /// 16-bit RGB (alpha was fully opaque and dropped), pre-serialised big-endian.
    Rgb16Be(Vec<u8>),
    /// 8-bit RGB plus a `tRNS` colour key (§11.3.2.1): the alpha channel was binary, every
    /// transparent pixel shared one colour, and no opaque pixel used it, so that colour can stand
    /// for "transparent" and the fourth channel disappears.
    Rgb8Keyed {
        /// One RGB triple per pixel.
        samples: Vec<u8>,
        /// The colour a decoder must render as fully transparent.
        key: [u8; 3],
    },
    /// Greyscale plus a `tRNS` colour key — the greyscale twin of [`Reduced::Rgb8Keyed`]. Always
    /// depth 8: a sub-byte depth would have to scale the key too, and the saving over depth 8 is
    /// smaller than the risk of getting that wrong.
    GrayKeyed {
        /// One grey sample per pixel.
        samples: Vec<u8>,
        /// The grey value a decoder must render as fully transparent.
        key: u8,
    },
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

/// Zeroes the colour channels of every fully transparent pixel, leaving alpha alone. Returns
/// `None` when the image has no fully transparent pixel to clean.
///
/// Nothing a decoder renders changes: at `alpha == 0` the colour channels are invisible by
/// definition. What changes is how well the image *compresses*, in three compounding ways:
///
/// 1. Transparent pixels all become identical, so `Sub` and `Paeth` filter a run of them to
///    zeros instead of to whatever noise the source happened to carry.
/// 2. [`analyze8`] keys its palette on the whole RGBA quad, so two invisible pixels that differ
///    only in their unseen colour cost two palette entries today. This collapses every
///    transparent pixel to a single entry.
/// 3. It is the precondition for a `tRNS` colour key, which needs one colour to stand for
///    "transparent".
///
/// One constant, not the neighbouring pixel's colour, and that choice was measured rather than
/// assumed. Inheriting the predecessor flattens a *run* just as well, but leaves every invisible
/// pixel a distinct RGBA quad, so (2) and (3) both fail: on an image alternating visible and
/// invisible pixels it collapsed nothing at all and saved zero bytes.
///
/// This is *not* lossless in the strict byte sense the rest of this module keeps -- the stored
/// samples change -- which is why it is opt-in via
/// [`PngEncoder::with_transparent_cleanup`](crate::PngEncoder::with_transparent_cleanup) and off
/// by default. `channels` must be 2 (grey + alpha) or 4 (RGBA); layouts without an alpha channel
/// have nothing to clean and return `None`.
pub(crate) fn clean_transparent(pixels: &[u8], channels: usize) -> Option<Vec<u8>> {
    debug_assert!((1..=4).contains(&channels));
    if !channels.is_multiple_of(2) {
        return None; // no alpha channel
    }
    let colour = channels - 1; // colour channels are everything before alpha
    if !pixels.chunks_exact(channels).any(|px| px[colour] == 0) {
        return None;
    }

    let mut out = pixels.to_vec();
    for px in out.chunks_exact_mut(channels) {
        if px[colour] == 0 {
            px[..colour].fill(0);
        }
    }
    Some(out)
}

/// The RGBA quad a pixel of any supported layout presents: grey replicates into R=G=B, and layouts
/// without an alpha channel (the odd channel counts) are opaque.
fn pixel_key(px: &[u8], channels: usize) -> [u8; 4] {
    let alpha = if channels.is_multiple_of(2) {
        px[channels - 1]
    } else {
        255
    };
    if channels >= 3 {
        [px[0], px[1], px[2], alpha]
    } else {
        [px[0], px[0], px[0], alpha]
    }
}

/// The colour that can stand for "transparent", if a `tRNS` colour key applies at all.
///
/// Three conditions, all necessary (§11.3.2.1 gives a decoder exactly one transparent colour, not
/// a mask):
///
/// 1. every alpha is 0 or 255 — a partially transparent pixel cannot be expressed by a key;
/// 2. at least one pixel is transparent — otherwise the plain alpha *drop* already applies and is
///    strictly better, since it costs no chunk;
/// 3. every transparent pixel shares one colour, and **no opaque pixel uses it** — otherwise the
///    key would erase a pixel that should be visible.
///
/// Condition 3 is why
/// [`PngEncoder::with_transparent_cleanup`](crate::PngEncoder::with_transparent_cleanup) pairs
/// with this: it collapses every invisible pixel to one colour, which is precisely what a key
/// needs. Without it, a source whose transparent pixels carry different unseen colours has no key
/// available and keeps its alpha channel.
///
/// Two passes rather than one: the candidate is not known until the first transparent pixel is
/// seen, so proving no *earlier* opaque pixel used it needs a second look. The second pass only
/// runs when the first has already established a candidate.
fn colour_key(pixels: &[u8], channels: usize) -> Option<[u8; 4]> {
    debug_assert!(channels == 2 || channels == 4);
    let mut candidate: Option<[u8; 4]> = None;
    let mut any_transparent = false;
    for px in pixels.chunks_exact(channels) {
        let key = pixel_key(px, channels);
        match key[3] {
            0 => {
                any_transparent = true;
                match candidate {
                    // A second transparent colour: no single key can stand for both.
                    Some(seen) if seen[..3] != key[..3] => return None,
                    Some(_) => {}
                    None => candidate = Some(key),
                }
            }
            255 => {}
            // Partial transparency cannot be expressed as a colour key.
            _ => return None,
        }
    }
    if !any_transparent {
        return None;
    }
    let candidate = candidate?;
    // The key must name a colour nothing visible uses.
    let collides = pixels.chunks_exact(channels).any(|px| {
        let key = pixel_key(px, channels);
        key[3] == 255 && key[..3] == candidate[..3]
    });
    (!collides).then_some(candidate)
}

/// Analyses interleaved 8-bit samples (`channels`: 1 = grey, 2 = grey+alpha, 3 = RGB, 4 = RGBA)
/// and returns the smallest lossless reduction that beats the input encoding, or `None` to keep it
/// as-is.
pub fn analyze8(pixels: &[u8], channels: usize) -> Option<Reduced> {
    debug_assert!((1..=4).contains(&channels));
    let pixel_count = pixels.len() / channels;

    let mut all_opaque = true;
    let mut all_gray = true;
    // Whether every grey value is exactly representable at 1/2/4 bits — a multiple of the depth's
    // §13.12 scale factor, not merely low-cardinality. Meaningful only while `all_gray` holds.
    let (mut fits1, mut fits2, mut fits4) = (true, true, true);
    let mut palette_index: HashMap<[u8; 4], u8> = HashMap::new();
    let mut palette: Vec<[u8; 4]> = Vec::new();
    let mut too_many_colors = false;
    for px in pixels.chunks_exact(channels) {
        let key = pixel_key(px, channels);
        all_opaque &= key[3] == 255;
        all_gray &= key[0] == key[1] && key[1] == key[2];
        fits1 &= key[0] == 0 || key[0] == 255;
        fits2 &= key[0].is_multiple_of(85);
        fits4 &= key[0].is_multiple_of(17);
        if !too_many_colors && let Entry::Vacant(slot) = palette_index.entry(key) {
            if palette.len() == 256 {
                too_many_colors = true;
            } else {
                slot.insert(palette.len() as u8);
                palette.push(key);
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
    let gray_depth = if fits1 {
        1
    } else if fits2 {
        2
    } else if fits4 {
        4
    } else {
        8
    };
    let gray_size = if all_gray && all_opaque {
        pixel_count * gray_depth as usize / 8
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
    // A colour key costs one `tRNS` chunk -- 6 bytes of payload for truecolour, 2 for greyscale,
    // plus 12 of framing -- and buys the whole alpha channel. Only worth looking for when alpha is
    // actually carrying something, which `all_opaque` already rules out.
    let key = if all_opaque || !channels.is_multiple_of(2) {
        None
    } else {
        colour_key(pixels, channels)
    };
    let keyed_size = match key {
        Some(_) if all_gray => pixel_count + 14,
        Some(_) => pixel_count * 3 + 18,
        None => usize::MAX,
    };

    let best = palette_size
        .min(gray_size)
        .min(gray_alpha_size)
        .min(rgb_size)
        .min(keyed_size);
    if best >= input_size {
        return None; // no reduction is smaller
    }

    if best == gray_size {
        let scale = gray8_scale(gray_depth);
        Some(Reduced::Gray {
            depth: gray_depth,
            samples: pixels
                .chunks_exact(channels)
                .map(|px| px[0] / scale)
                .collect(),
        })
    } else if best == gray_alpha_size {
        let mut out = Vec::with_capacity(pixel_count * 2);
        for px in pixels.chunks_exact(channels) {
            let key = pixel_key(px, channels);
            out.push(key[0]);
            out.push(key[3]);
        }
        Some(Reduced::GrayAlpha8(out))
    } else if best == keyed_size {
        let key = key.expect("keyed_size is only finite when a key was found");
        if all_gray {
            Some(Reduced::GrayKeyed {
                samples: pixels
                    .chunks_exact(channels)
                    .map(|px| pixel_key(px, channels)[0])
                    .collect(),
                key: key[0],
            })
        } else {
            let mut out = Vec::with_capacity(pixel_count * 3);
            for px in pixels.chunks_exact(channels) {
                out.extend_from_slice(&pixel_key(px, channels)[0..3]);
            }
            Some(Reduced::Rgb8Keyed {
                samples: out,
                key: [key[0], key[1], key[2]],
            })
        }
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

/// Analyses interleaved 16-bit samples (`channels` as in [`analyze8`]). An input where every
/// sample's high byte equals its low byte (`v == k·257`, the exact inverse of the decoder's 8→16
/// widening) is demoted and re-analysed at 8 bits — the demotion alone halves the payload, so it
/// always reduces. Otherwise only the 16-bit-native channel reductions (grey, alpha drop) apply;
/// PNG has no 16-bit palette.
pub fn analyze16(samples: &[u16], channels: usize) -> Option<Reduced> {
    debug_assert!((1..=4).contains(&channels));
    if let Some(demoted) = demote16(samples) {
        let further = analyze8(&demoted, channels);
        return Some(further.unwrap_or(match channels {
            1 => Reduced::Gray {
                depth: 8,
                samples: demoted,
            },
            2 => Reduced::GrayAlpha8(demoted),
            3 => Reduced::Rgb8(demoted),
            _ => Reduced::Rgba8(demoted),
        }));
    }

    let mut all_opaque = true;
    let mut all_gray = true;
    for px in samples.chunks_exact(channels) {
        if channels.is_multiple_of(2) {
            all_opaque &= px[channels - 1] == u16::MAX;
        }
        if channels >= 3 {
            all_gray &= px[0] == px[1] && px[1] == px[2];
        }
    }

    // Unlike the 8-bit analysis there is no size estimate to weigh: the candidates' gates are
    // mutually exclusive, and each strictly shrinks the channel count, so whichever gate matches
    // wins outright. The channel checks reject the identity "reductions" (grey of a Gray16 input,
    // grey+alpha of a GrayAlpha16 input).
    let px16 = samples.chunks_exact(channels);
    if all_gray && all_opaque && channels > 1 {
        Some(Reduced::Gray16Be(be_bytes(px16.map(|px| px[0]))))
    } else if all_gray && channels > 2 {
        // Not all-opaque (that is the branch above), so the alpha channel must be kept.
        Some(Reduced::GrayAlpha16Be(be_bytes(
            px16.flat_map(|px| [px[0], px[channels - 1]]),
        )))
    } else if channels == 4 && all_opaque {
        // Not all-grey (the branches above), so only the opaque alpha channel can be dropped.
        Some(Reduced::Rgb16Be(be_bytes(
            px16.flat_map(|px| [px[0], px[1], px[2]]),
        )))
    } else {
        None
    }
}

/// The 8-bit demotion of `samples`, or `None` unless every sample is exactly `k·257` (high byte ==
/// low byte), which makes the demotion reversible by ×257 widening.
fn demote16(samples: &[u16]) -> Option<Vec<u8>> {
    samples
        .iter()
        .map(|&v| {
            let [hi, lo] = v.to_be_bytes();
            (hi == lo).then_some(lo)
        })
        .collect()
}

/// Serialises 16-bit samples big-endian (PNG's network byte order).
fn be_bytes(samples: impl Iterator<Item = u16>) -> Vec<u8> {
    samples.flat_map(u16::to_be_bytes).collect()
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
        .map(|px| *palette_index.get(&pixel_key(px, channels)).unwrap_or(&0))
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
    fn cleaning_declines_when_there_is_nothing_invisible_to_clean() {
        // `None` rather than an unchanged copy: the encoder must be able to tell "no work" from
        // "work that happened to change nothing", or it allocates a whole image for nothing.
        let opaque: Vec<u8> = (0..16u8).flat_map(|i| [i, i + 1, i + 2, 255]).collect();
        assert!(clean_transparent(&opaque, 4).is_none());

        // Layouts with no alpha channel have nothing to clean, whatever the samples say.
        assert!(clean_transparent(&opaque, 3).is_none());
        assert!(clean_transparent(&opaque, 1).is_none());
    }

    #[test]
    fn cleaning_zeroes_invisible_colour_and_leaves_everything_else() {
        let src: Vec<u8> = vec![
            10, 20, 30, 255, // visible
            40, 50, 60, 0, // invisible: colour must go
            70, 80, 90, 128, // partially transparent: still visible, must stay
        ];
        let cleaned = clean_transparent(&src, 4).expect("there is a transparent pixel");
        assert_eq!(
            cleaned,
            vec![
                10, 20, 30, 255, //
                0, 0, 0, 0, //
                70, 80, 90, 128,
            ]
        );
    }

    #[test]
    fn cleaning_grey_alpha_zeroes_only_the_grey_channel() {
        let src: Vec<u8> = vec![200, 255, 111, 0, 90, 1];
        let cleaned = clean_transparent(&src, 2).expect("there is a transparent pixel");
        assert_eq!(cleaned, vec![200, 255, 0, 0, 90, 1]);
    }

    #[test]
    fn drops_opaque_alpha() {
        // Opaque, non-grey RGBA -> RGB.
        let rgba = [10, 20, 30, 255, 40, 50, 60, 255];
        match analyze8(&rgba, 4) {
            Some(Reduced::Rgb8(rgb)) => assert_eq!(rgb, vec![10, 20, 30, 40, 50, 60]),
            _ => panic!("expected Rgb8"),
        }
    }

    #[test]
    fn detects_grayscale() {
        // Opaque R=G=B RGB with many levels -> 8-bit grey.
        let rgb: Vec<u8> = (0..60u8).flat_map(|v| [v, v, v]).collect();
        match analyze8(&rgb, 3) {
            Some(Reduced::Gray { depth: 8, samples }) => {
                assert_eq!(samples, (0..60u8).collect::<Vec<_>>());
            }
            _ => panic!("expected 8-bit Gray"),
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
        match analyze8(&rgb, 3) {
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
        assert!(analyze8(&rgb, 3).is_none());
    }

    #[test]
    fn palette_with_transparency_emits_trns() {
        let rgba = [
            0, 0, 0, 0, // transparent black
            255, 255, 255, 255, // opaque white
        ]
        .repeat(20);
        match analyze8(&rgba, 4) {
            Some(Reduced::Indexed { trns: Some(t), .. }) => assert_eq!(t, vec![0]),
            _ => panic!("expected indexed with tRNS"),
        }
    }

    /// Asserts `pixels` (of `channels`) reduces to grey at `depth` with the expected codes.
    fn expect_gray(pixels: &[u8], channels: usize, depth: u8, codes: &[u8]) {
        match analyze8(pixels, channels) {
            Some(Reduced::Gray {
                depth: got,
                samples,
            }) => {
                assert_eq!(got, depth, "depth");
                assert_eq!(samples, codes, "codes");
            }
            _ => panic!("expected Gray at depth {depth}"),
        }
    }

    #[test]
    fn grey_packs_to_the_smallest_exact_depth() {
        // §13.12 exactness per depth: 1-bit {0,255}, 2-bit multiples of 85, 4-bit multiples of 17.
        // Each set breaks the next-lower depth's divisibility, pinning the moduli individually.
        let bw: Vec<u8> = [0u8, 255].repeat(30);
        expect_gray(&bw, 1, 1, &[0, 1].repeat(30));

        let quarters: Vec<u8> = [0u8, 85, 170, 255].repeat(20);
        expect_gray(&quarters, 1, 2, &[0, 1, 2, 3].repeat(20));

        let sixteenths: Vec<u8> = (0..16u8).map(|v| v * 17).collect::<Vec<_>>().repeat(10);
        expect_gray(&sixteenths, 1, 4, &(0..16u8).collect::<Vec<_>>().repeat(10));

        // The same values arriving as opaque RGBA reduce straight to sub-byte grey too.
        let rgba: Vec<u8> = quarters.iter().flat_map(|&v| [v, v, v, 255]).collect();
        expect_gray(&rgba, 4, 2, &[0, 1, 2, 3].repeat(20));
    }

    #[test]
    fn low_cardinality_grey_off_the_scale_grid_is_indexed() {
        // Three grey levels, none a §13.12 multiple: not packable as grey, but a grey palette at
        // 2 bits still beats 8-bit grey.
        let gray: Vec<u8> = [5u8, 9, 200].repeat(40);
        match analyze8(&gray, 1) {
            Some(Reduced::Indexed {
                depth, plte, trns, ..
            }) => {
                assert_eq!(depth, 2);
                assert_eq!(plte, vec![5, 5, 5, 9, 9, 9, 200, 200, 200]);
                assert!(trns.is_none());
            }
            _ => panic!("expected Indexed"),
        }
    }

    #[test]
    fn grey_input_with_full_range_keeps_its_encoding() {
        // 8-bit grey using values off every sub-byte grid and >16 distinct levels: nothing beats
        // the input.
        let gray: Vec<u8> = (0..=255u8).collect();
        assert!(analyze8(&gray, 1).is_none());
    }

    #[test]
    fn grey_alpha_drops_an_opaque_alpha_channel() {
        let ga: Vec<u8> = (0..90u8).flat_map(|v| [v, 255]).collect();
        expect_gray(&ga, 2, 8, &(0..90u8).collect::<Vec<_>>());
    }

    #[test]
    fn grey_alpha_with_few_combinations_is_indexed_with_trns() {
        let ga: Vec<u8> = [0, 0, 255, 255].repeat(40); // transparent black, opaque white
        match analyze8(&ga, 2) {
            Some(Reduced::Indexed {
                depth,
                plte,
                trns: Some(t),
                ..
            }) => {
                assert_eq!(depth, 1);
                assert_eq!(plte, vec![0, 0, 0, 255, 255, 255]);
                assert_eq!(t, vec![0]);
            }
            _ => panic!("expected indexed with tRNS"),
        }
    }

    #[test]
    fn grey_alpha_noise_keeps_its_encoding() {
        let ga: Vec<u8> = (0..600u32)
            .flat_map(|i| [(i % 251) as u8, (i % 249) as u8])
            .collect();
        assert!(analyze8(&ga, 2).is_none());
    }

    #[test]
    fn demotable_sixteen_bit_recurses_into_the_eight_bit_analysis() {
        // Grey, opaque, every sample k*257 -> demoted and reduced all the way to 8-bit grey.
        let rgba16: Vec<u16> = (0..80u16)
            .flat_map(|i| {
                let v = (i % 60) * 257;
                [v, v, v, u16::MAX]
            })
            .collect();
        match analyze16(&rgba16, 4) {
            Some(Reduced::Gray { depth: 8, samples }) => {
                assert_eq!(samples, (0..80).map(|i| (i % 60) as u8).collect::<Vec<_>>());
            }
            _ => panic!("expected 8-bit Gray"),
        }
    }

    #[test]
    fn demotable_but_irreducible_sixteen_bit_falls_back_to_plain_demotion() {
        // Every sample k*257 but many colours and varied alpha: the demotion itself is the win.
        let rgba16: Vec<u16> = (0..600u32)
            .flat_map(|i| {
                [
                    ((i % 251) * 257) as u16,
                    ((i % 241) * 257) as u16,
                    ((i % 239) * 257) as u16,
                    ((i % 233) * 257) as u16,
                ]
            })
            .collect();
        match analyze16(&rgba16, 4) {
            Some(Reduced::Rgba8(demoted)) => {
                assert_eq!(demoted.len(), 600 * 4);
                assert_eq!(demoted[0..4], [0, 0, 0, 0]);
                assert_eq!(demoted[4 * 250], 250u8);
            }
            _ => panic!("expected demoted Rgba8"),
        }
    }

    #[test]
    fn two_matching_channels_are_not_grey() {
        // R == G but B differs on every pixel: not greyscale, too many colours to palette.
        let rgb: Vec<u8> = (0..60u8).flat_map(|v| [v, v, 200]).collect();
        assert!(analyze8(&rgb, 3).is_none());
        // The 16-bit twin (non-demotable): same verdict.
        let rgb16: Vec<u16> = (0..60u32)
            .flat_map(|i| {
                let v = (i * 501 + 1) as u16;
                [v, v, 200]
            })
            .collect();
        assert!(analyze16(&rgb16, 3).is_none());
    }

    #[test]
    fn sixteen_bit_identity_reductions_are_rejected() {
        // Non-demotable grey noise arriving as Gray16 is already minimal.
        let gray16: Vec<u16> = (0..90u32).map(|i| (i * 501 + 1) as u16).collect();
        assert!(analyze16(&gray16, 1).is_none());
    }

    #[test]
    fn demotable_grey_alpha_and_rgb_fall_back_to_their_own_layout() {
        // GrayAlpha16, every sample k*257, varied alpha, >256 (grey, alpha) combos: the demotion
        // itself is the only win, and it must keep the grey+alpha layout.
        let ga16: Vec<u16> = (0..600u32)
            .flat_map(|i| [((i % 251) * 257) as u16, ((i % 33) * 7 * 257) as u16])
            .collect();
        match analyze16(&ga16, 2) {
            Some(Reduced::GrayAlpha8(demoted)) => assert_eq!(demoted.len(), 600 * 2),
            _ => panic!("expected demoted GrayAlpha8"),
        }

        // Rgb16, every sample k*257, many non-grey colours: plain demotion keeps RGB.
        let rgb16: Vec<u16> = (0..600u32)
            .flat_map(|i| {
                [
                    ((i % 251) * 257) as u16,
                    ((i % 241) * 257) as u16,
                    ((i % 239) * 257) as u16,
                ]
            })
            .collect();
        match analyze16(&rgb16, 3) {
            Some(Reduced::Rgb8(demoted)) => assert_eq!(demoted.len(), 600 * 3),
            _ => panic!("expected demoted Rgb8"),
        }
    }

    #[test]
    fn one_asymmetric_sample_disables_demotion() {
        // All grey/opaque k*257 except a single 0x0100 (hi != lo): must stay 16-bit native.
        let mut rgba16: Vec<u16> = (0..80u16)
            .flat_map(|i| {
                let v = (i % 60) * 257;
                [v, v, v, u16::MAX]
            })
            .collect();
        rgba16[0] = 0x0100;
        rgba16[1] = 0x0100;
        rgba16[2] = 0x0100; // keep the pixel grey so the native grey reduction still applies
        match analyze16(&rgba16, 4) {
            Some(Reduced::Gray16Be(bytes)) => {
                assert_eq!(&bytes[0..2], &[0x01, 0x00], "big-endian, undemoted");
            }
            _ => panic!("expected Gray16Be"),
        }
    }

    #[test]
    fn sixteen_bit_native_reductions_serialise_big_endian() {
        // Opaque, non-grey, non-demotable RGBA16 -> RGB16, big-endian.
        let rgba16: Vec<u16> = (0..90u32)
            .flat_map(|i| [(i * 501 + 1) as u16, (i * 703 + 2) as u16, 3, u16::MAX])
            .collect();
        match analyze16(&rgba16, 4) {
            Some(Reduced::Rgb16Be(bytes)) => {
                assert_eq!(bytes.len(), 90 * 6);
                assert_eq!(&bytes[0..6], &[0, 1, 0, 2, 0, 3]);
            }
            _ => panic!("expected Rgb16Be"),
        }

        // Grey non-demotable RGB16 -> Gray16.
        let rgb16: Vec<u16> = (0..90u32)
            .flat_map(|i| {
                let v = (i * 501 + 1) as u16;
                [v, v, v]
            })
            .collect();
        assert!(matches!(
            analyze16(&rgb16, 3),
            Some(Reduced::Gray16Be(bytes)) if bytes.len() == 90 * 2
        ));

        // Opaque non-demotable GrayAlpha16 -> Gray16.
        let ga16: Vec<u16> = (0..90u32)
            .flat_map(|i| [(i * 501 + 1) as u16, u16::MAX])
            .collect();
        assert!(matches!(
            analyze16(&ga16, 2),
            Some(Reduced::Gray16Be(bytes)) if bytes.len() == 90 * 2
        ));

        // A translucent grey+alpha pair -> gets a full 16-bit gray+alpha only when smaller, which
        // it never is for GrayAlpha16 input; and 16-bit noise stays as-is.
        let translucent: Vec<u16> = (0..90u32)
            .flat_map(|i| [(i * 501 + 1) as u16, (i * 703) as u16 | 1])
            .collect();
        assert!(analyze16(&translucent, 2).is_none());
        let noise: Vec<u16> = (0..600u32)
            .flat_map(|i| {
                [
                    (i * 501 + 1) as u16,
                    (i * 703 + 2) as u16,
                    (i * 907 + 3) as u16,
                    (i * 111) as u16 | 1,
                ]
            })
            .collect();
        assert!(analyze16(&noise, 4).is_none());
    }

    #[test]
    fn translucent_rgba16_reduces_to_grey_alpha() {
        // Grey with varied (non-demotable) alpha -> GrayAlpha16Be at 4 bytes/px vs 8.
        let rgba16: Vec<u16> = (0..90u32)
            .flat_map(|i| {
                let v = (i * 501 + 1) as u16;
                [v, v, v, (i * 703) as u16 | 1]
            })
            .collect();
        match analyze16(&rgba16, 4) {
            Some(Reduced::GrayAlpha16Be(bytes)) => assert_eq!(bytes.len(), 90 * 4),
            _ => panic!("expected GrayAlpha16Be"),
        }
    }
}
