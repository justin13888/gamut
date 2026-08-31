//! Corpus plumbing for the real-camera DNG conformance tier (gamut issue #174).
//!
//! The corpus itself lives in the `gamut-dng-samples` submodule at `third_party/gamut-dng-samples`
//! — real files written by real cameras, every one CC0. This crate resolves that directory, parses
//! the corpus `MANIFEST.toml`, and hands the tests a typed list of samples plus the properties
//! `gamut-dng` must observe for each.
//!
//! Expectations live in the manifest rather than in Rust so that adding a sample is a change to the
//! samples repository alone.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The parsed corpus manifest: provenance plus per-file expectations.
#[derive(Debug, Deserialize)]
pub struct Manifest {
    /// Every sample in the corpus, in manifest order.
    #[serde(rename = "sample")]
    pub samples: Vec<Sample>,
}

/// One corpus file: where it came from, and what decoding it must produce.
#[derive(Debug, Deserialize)]
pub struct Sample {
    /// Corpus-relative path, e.g. `apple/iphone-12-pro/IMG_1361.DNG`.
    pub path: String,
    /// SHA-256 of the file as committed, matching the upstream repository index.
    pub sha256: String,
    /// SPDX licence identifier; every corpus file is `CC0-1.0`.
    pub license: String,
    /// Upstream URL the file was fetched from.
    pub source: String,
    /// Human-readable camera name, for assertion messages.
    pub camera: String,
    /// One line on what this file — and only this file — proves.
    pub covers: String,
    /// What `gamut-dng` must observe for this file.
    pub expect: Expect,
}

/// The observable properties a corpus file pins down. Every value is *measured* from the file,
/// so a change in `gamut-dng`'s behaviour fails the tier rather than passing quietly.
#[derive(Debug, Deserialize)]
pub struct Expect {
    /// `DNGVersion` rendered as `a.b.c.d`.
    pub dng_version: String,
    /// Container byte order: `little` or `big`.
    pub byte_order: String,
    /// Raw image dimensions, `[width, height]`.
    pub dims: [u32; 2],
    /// Bits per sample of the reconstructed raw image.
    pub bits: u16,
    /// Samples per pixel of the raw image.
    pub samples: u16,
    /// Raw photometry: `Cfa` or `LinearRaw`.
    pub photometry: String,
    /// The raw IFD's compression scheme, by enum name.
    pub compression: String,
    /// Tile geometry `[width, length]` when the raw image is tiled; absent when stripped.
    pub tiled: Option<[u32; 2]>,
    /// Number of chunks (tiles or strips) the raw image is stored in.
    pub chunks: usize,
    /// Number of non-raw image IFDs surfaced as sub-images.
    pub sub_images: usize,
    /// How many of those fall back to verbatim chunks (deferred payloads, which must not error).
    pub undecoded_sub_images: usize,
    /// Whether the raw IFD carries a `ProfileGainTableMap`.
    pub gain_table_map: bool,
    /// Whether the file carries colour calibration at all. `false` for a monochrome DNG, which
    /// must decode with no profile rather than an invented one.
    pub profile: bool,
    /// Whether `DngDecoder::decode` must succeed.
    pub decodes: bool,
    /// For `decodes = false`, the required `ErrorKind` — a deferred feature must be a *typed*
    /// refusal, never a panic or wrong pixels.
    pub error_kind: Option<String>,
    /// Whether `DngRewrite` must round-trip this file.
    pub rewritable: bool,
    /// The required `MakerNotePreservation` outcome: `Absent`, `Pinned` or `Relocated`.
    pub maker_note: String,
    /// Expected `DigestCheck` verdict: `absent` or `match`.
    pub raw_digest: String,
    /// How many byte runs the file's own structures do not account for.
    pub unaccounted_spans: usize,
    /// How many bytes those runs cover in total.
    pub unaccounted_bytes: u64,
}

/// Returns the corpus directory (the `gamut-dng-samples` submodule checkout).
///
/// # Panics
///
/// Panics when the submodule is not checked out, naming the command that fixes it. The corpus is
/// the entire point of this crate, so an absent corpus is a hard error rather than a silent skip —
/// matching the workspace's no-skip posture.
#[must_use]
pub fn corpus_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../third_party/gamut-dng-samples");
    assert!(
        dir.join("MANIFEST.toml").is_file(),
        "real-camera corpus missing at {}\nfetch it with: mise run fetch-dng-samples",
        dir.display()
    );
    dir
}

/// Parses the corpus `MANIFEST.toml`.
///
/// # Panics
///
/// Panics when the manifest cannot be read or does not parse — both are corpus bugs, not
/// properties of any file under test.
#[must_use]
pub fn manifest() -> Manifest {
    let path = corpus_dir().join("MANIFEST.toml");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    toml::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// SHA-256 of `data`, for checking a corpus file against the checksum its manifest publishes.
///
/// Implemented here rather than pulled in as a dependency: this crate is dev-only and excluded
/// from the workspace, and the hash is needed for exactly one purpose.
#[must_use]
pub fn sha256(data: &[u8]) -> [u8; 32] {
    // FIPS 180-4 round constants and initial hash values, laid out in the spec's own rows.
    // Skipped because every element is 11 chars, one over rustfmt's
    // `short_array_element_width_threshold`, which would otherwise break them one per line.
    #[rustfmt::skip]
    const K: [u32; 64] = [
        0x428a_2f98, 0x7137_4491, 0xb5c0_fbcf, 0xe9b5_dba5, 0x3956_c25b, 0x59f1_11f1, 0x923f_82a4,
        0xab1c_5ed5, 0xd807_aa98, 0x1283_5b01, 0x2431_85be, 0x550c_7dc3, 0x72be_5d74, 0x80de_b1fe,
        0x9bdc_06a7, 0xc19b_f174, 0xe49b_69c1, 0xefbe_4786, 0x0fc1_9dc6, 0x240c_a1cc, 0x2de9_2c6f,
        0x4a74_84aa, 0x5cb0_a9dc, 0x76f9_88da, 0x983e_5152, 0xa831_c66d, 0xb003_27c8, 0xbf59_7fc7,
        0xc6e0_0bf3, 0xd5a7_9147, 0x06ca_6351, 0x1429_2967, 0x27b7_0a85, 0x2e1b_2138, 0x4d2c_6dfc,
        0x5338_0d13, 0x650a_7354, 0x766a_0abb, 0x81c2_c92e, 0x9272_2c85, 0xa2bf_e8a1, 0xa81a_664b,
        0xc24b_8b70, 0xc76c_51a3, 0xd192_e819, 0xd699_0624, 0xf40e_3585, 0x106a_a070, 0x19a4_c116,
        0x1e37_6c08, 0x2748_774c, 0x34b0_bcb5, 0x391c_0cb3, 0x4ed8_aa4a, 0x5b9c_ca4f, 0x682e_6ff3,
        0x748f_82ee, 0x78a5_636f, 0x84c8_7814, 0x8cc7_0208, 0x90be_fffa, 0xa450_6ceb, 0xbef9_a3f7,
        0xc671_78f2,
    ];
    #[rustfmt::skip]
    let mut h: [u32; 8] = [
        0x6a09_e667, 0xbb67_ae85, 0x3c6e_f372, 0xa54f_f53a, 0x510e_527f, 0x9b05_688c, 0x1f83_d9ab,
        0x5be0_cd19,
    ];

    let mut message = data.to_vec();
    let bit_len = (data.len() as u64) * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    for block in message.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, word) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(v);
        }
    }

    let mut out = [0u8; 32];
    for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(h) {
        chunk.copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two canonical NIST vectors, so a corpus checksum mismatch is never the hash's fault.
    #[test]
    fn sha256_matches_the_known_vectors() {
        let hex = |d: [u8; 32]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(
            hex(sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
