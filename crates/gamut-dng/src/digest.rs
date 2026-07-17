//! The DNG raw-image digest (`NewRawImageDigest`, tag 51111): the Adobe SDK's
//! MD5-over-raw-image algorithm (`dng_negative::FindNewRawImageDigest`), reproduced bit-exactly
//! and differentially gated against the SDK.
//!
//! The algorithm (SDK `dng_find_new_raw_image_digest_task`):
//!
//! 1. The image is covered by a grid of **digest tiles** whose unit cell is
//!    `min(256, height) × min(256, width)`; edge tiles are clipped to the image, not padded.
//! 2. Each tile's samples are serialised **planar** (all of plane 0's tile pixels row-major,
//!    then plane 1, …) as little-endian `u16` — or as single bytes when a `LinearizationTable`
//!    with ≤ 256 entries is present (the SDK stores such data 8-bit) — and MD5-hashed.
//! 3. The final digest is the MD5 of the tile digests concatenated in row-major tile order.
//!
//! The SDK folds a raw transparency mask into the digest when one exists; gamut-dng never
//! writes one, so that branch is not modelled.

use crate::md5::md5;
use crate::raw::RawImage;

/// Computes the `NewRawImageDigest` of `raw` (see the module docs).
#[must_use]
pub(crate) fn new_raw_image_digest(raw: &RawImage) -> [u8; 16] {
    let width = raw.dimensions().width as usize;
    let height = raw.dimensions().height as usize;
    let spp = usize::from(raw.samples_per_pixel());
    let samples = raw.samples();

    // With a <= 256-entry linearization table the SDK digests the data as bytes.
    let byte_mode = raw
        .levels()
        .linearization_table()
        .is_some_and(|table| table.len() <= 256);

    let cell_w = width.min(256);
    let cell_h = height.min(256);
    let across = width.div_ceil(cell_w);
    let down = height.div_ceil(cell_h);

    // No capacity hints: `tile_bytes` reaches steady-state capacity after the first tile (it is
    // reused via `clear`), and hint arithmetic would only breed unkillable capacity mutants.
    let mut tile_digests = Vec::new();
    let mut tile_bytes = Vec::new();
    for tile_row in 0..down {
        for tile_col in 0..across {
            let x0 = tile_col * cell_w;
            let y0 = tile_row * cell_h;
            let tile_w = cell_w.min(width - x0);
            let tile_h = cell_h.min(height - y0);
            tile_bytes.clear();
            for plane in 0..spp {
                for row in 0..tile_h {
                    for col in 0..tile_w {
                        let sample = samples[((y0 + row) * width + x0 + col) * spp + plane];
                        if byte_mode {
                            tile_bytes.push(sample as u8);
                        } else {
                            tile_bytes.extend_from_slice(&sample.to_le_bytes());
                        }
                    }
                }
            }
            tile_digests.extend_from_slice(&md5(&tile_bytes));
        }
    }
    md5(&tile_digests)
}

#[cfg(test)]
mod tests {
    use gamut_core::Dimensions;

    use super::*;

    fn raw(width: u32, height: u32, spp: u16) -> RawImage {
        let n = width as usize * height as usize * usize::from(spp);
        let samples: Vec<u16> = (0..n).map(|i| (i * 37 % 4096) as u16).collect();
        if spp == 1 {
            RawImage::new_cfa(
                Dimensions::new(width, height).unwrap(),
                12,
                (2, 2),
                vec![0, 1, 1, 2],
                samples,
            )
            .unwrap()
        } else {
            RawImage::new_linear_raw(Dimensions::new(width, height).unwrap(), 12, spp, samples)
                .unwrap()
        }
    }

    /// Hand-computed golden for a single-tile grayscale image: the digest is
    /// `md5(md5(le_bytes(samples)))` — pinned against an independent reference computation, so
    /// the planar order, endianness, and two-level hashing cannot silently change.
    #[test]
    fn single_tile_digest_is_md5_of_tile_md5() {
        let raw = raw(4, 3, 1);
        let mut bytes = Vec::new();
        for &s in raw.samples() {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        let expected = md5(&md5(&bytes));
        assert_eq!(new_raw_image_digest(&raw), expected);
    }

    /// Multi-plane serialisation is planar, not interleaved.
    #[test]
    fn multi_plane_digest_is_planar() {
        let raw = raw(3, 2, 3);
        let mut planar = Vec::new();
        for plane in 0..3usize {
            for pixel in 0..6usize {
                planar.extend_from_slice(&raw.samples()[pixel * 3 + plane].to_le_bytes());
            }
        }
        assert_eq!(new_raw_image_digest(&raw), md5(&md5(&planar)));
        // An interleaved serialisation would differ.
        let mut interleaved = Vec::new();
        for &s in raw.samples() {
            interleaved.extend_from_slice(&s.to_le_bytes());
        }
        assert_ne!(new_raw_image_digest(&raw), md5(&md5(&interleaved)));
    }

    /// A <= 256-entry linearization table switches the serialisation to bytes.
    #[test]
    fn small_linearization_table_digests_bytes() {
        let samples: Vec<u16> = (0..12).map(|i| i * 20).collect();
        let base = RawImage::new_cfa(
            Dimensions::new(4, 3).unwrap(),
            8,
            (2, 2),
            vec![0, 1, 1, 2],
            samples,
        )
        .unwrap();
        let table: Vec<u16> = (0..256).map(|v| v * 257).collect();
        let with_table = base
            .clone()
            .with_levels(
                crate::levels::RawLevels::uniform(1, 0.0, 255.0)
                    .unwrap()
                    .with_linearization_table(table),
            )
            .unwrap();
        let bytes: Vec<u8> = with_table.samples().iter().map(|&s| s as u8).collect();
        assert_eq!(new_raw_image_digest(&with_table), md5(&md5(&bytes)));
        assert_ne!(
            new_raw_image_digest(&with_table),
            new_raw_image_digest(&base),
            "the byte-mode digest must differ from the u16 digest"
        );
    }
}
