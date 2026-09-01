//! The efficiency corpus (issue #224): deterministic image generators shared by
//! `benches/encode.rs` and `tests/size_contract.rs`.
//!
//! One file, included by both, because the size contract's budgets are only meaningful if they
//! are measured on the same pixels the benchmark table reports. Two copies would drift, and the
//! drift would be invisible — a budget that no longer describes the row it names.
//!
//! Dependency-free on purpose: the benchmark includes it with `#[path]`, so it must not reach for
//! anything outside `core`/`alloc`.
//!
//! Each generator is one axis of encoder behaviour, and no two overlap. There is no vendored
//! image corpus in this crate (`README.md` says so), so every fixture is generated.

#![allow(dead_code)]

/// A deterministic, non-trivial RGB gradient — the workspace's shared bench pattern. Avoids the
/// all-constant fast paths so the measured work reflects realistic entropy.
pub fn gradient_rgb(side: u32) -> Vec<u8> {
    let mut buf = vec![0u8; (side * side * 3) as usize];
    for y in 0..side {
        for x in 0..side {
            let i = ((y * side + x) * 3) as usize;
            buf[i] = (x ^ y) as u8;
            buf[i + 1] = x.wrapping_mul(3).wrapping_add(y) as u8;
            buf[i + 2] = x.wrapping_add(y.wrapping_mul(7)) as u8;
        }
    }
    buf
}

/// Smooth, photograph-like content: three triangle waves at co-prime periods standing in for
/// sinusoids (no floating point in a fixture). Palette-hostile and 16-bit-hostile, so no reduction
/// applies and the whole residual is filtering plus DEFLATE — the row that measures the
/// compressor rather than the analysis.
pub fn photo_rgb(side: u32) -> Vec<u8> {
    let tri = |v: i64, period: i64| {
        let m = v.rem_euclid(period * 2);
        let up = if m < period { m } else { period * 2 - m };
        (up * 255 / period) as u8
    };
    let mut buf = vec![0u8; (side * side * 3) as usize];
    for y in 0..side {
        for x in 0..side {
            let i = ((y * side + x) * 3) as usize;
            let (xi, yi) = (i64::from(x), i64::from(y));
            buf[i] = tri(xi + yi, 61);
            buf[i + 1] = tri(xi * 2 - yi, 43);
            buf[i + 2] = tri(xi + yi * 3, 97);
        }
    }
    buf
}

/// Incompressible: a full avalanche mix of the byte index. Pins that the encoder does not
/// *expand* random data by more than stored-block framing, and drives `FilterType::None`.
///
/// Deliberately not the plain `i * 2654435761 >> 24` that `gamut-deflate`'s bench uses. Over a
/// dense index that top byte changes only once every few hundred `i`, so a "noise" row built that
/// way compresses roughly 97x and measures nothing at all.
pub fn noise_rgb(side: u32) -> Vec<u8> {
    (0..(side * side * 3))
        .map(|i: u32| {
            let mut v = i.wrapping_add(0x9E37_79B9);
            v ^= v >> 16;
            v = v.wrapping_mul(0x21F0_AAAD);
            v ^= v >> 15;
            v = v.wrapping_mul(0x735A_2D97);
            v ^= v >> 15;
            v as u8
        })
        .collect()
}

/// A greyscale ramp presented as RGB: R=G=B everywhere, so the grey reduction applies and two
/// channels disappear before DEFLATE runs.
pub fn grey_as_rgb(side: u32) -> Vec<u8> {
    let mut buf = vec![0u8; (side * side * 3) as usize];
    for y in 0..side {
        for x in 0..side {
            let i = ((y * side + x) * 3) as usize;
            let v = ((x + y) % 256) as u8;
            buf[i] = v;
            buf[i + 1] = v;
            buf[i + 2] = v;
        }
    }
    buf
}

/// Exactly 64 distinct colours over two alpha levels: the indexed + tRNS path, which is the
/// biggest structural lever this crate has over libpng-9 (libpng does not auto-palettise).
pub fn palette64_rgba(side: u32) -> Vec<u8> {
    let mut buf = vec![0u8; (side * side * 4) as usize];
    for y in 0..side {
        for x in 0..side {
            let i = ((y * side + x) * 4) as usize;
            let idx = ((x / 8 + y / 8 * 8) % 64) as u8;
            buf[i] = idx.wrapping_mul(4);
            buf[i + 1] = idx.wrapping_mul(9);
            buf[i + 2] = 255 - idx.wrapping_mul(3);
            buf[i + 3] = if idx.is_multiple_of(8) { 0 } else { 255 };
        }
    }
    buf
}

/// A sprite: binary alpha, where the fully transparent pixels carry *different* RGB values.
///
/// That invisible colour noise is what the palette build keys on today, so this is the only entry
/// that can see the dirty-alpha and tRNS-colour-key axes. It is the row to watch when either
/// lands.
pub fn sprite_rgba(side: u32) -> Vec<u8> {
    let mut buf = vec![0u8; (side * side * 4) as usize];
    let r2 = (i64::from(side) * i64::from(side)) / 9;
    for y in 0..side {
        for x in 0..side {
            let i = ((y * side + x) * 4) as usize;
            let cx = i64::from(x) - i64::from(side) / 2;
            let cy = i64::from(y) - i64::from(side) / 2;
            if cx * cx + cy * cy < r2 {
                buf[i] = (x ^ y) as u8;
                buf[i + 1] = 0x40;
                buf[i + 2] = 0xC0;
                buf[i + 3] = 255;
            } else {
                // Invisible, and deliberately not constant.
                buf[i] = x as u8;
                buf[i + 1] = y as u8;
                buf[i + 2] = (x ^ y) as u8;
                buf[i + 3] = 0;
            }
        }
    }
    buf
}

/// One fully opaque colour: the compressible extreme, where the whole reduce cascade applies and
/// chunk framing is most of what is left to measure.
pub fn flat_rgba(side: u32) -> Vec<u8> {
    (0..(side * side))
        .flat_map(|_| [0x2E, 0x86, 0xC1, 0xFF])
        .collect()
}
