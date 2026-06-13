//! Dynamic-Huffman DEFLATE blocks (BTYPE = 10, RFC 1951 §3.2.7).
//!
//! A dynamic block carries its own literal/length and distance codes, described by a sequence of
//! per-symbol code lengths that is itself run-length-coded (symbols 16/17/18) and Huffman-coded with
//! a small "code-length" alphabet. This is where most of DEFLATE's density comes from.

use crate::bitwriter::BitWriter;
use crate::huffman::CanonicalCode;
use crate::lz77::Token;
use crate::symbols;

/// Literal/length alphabet size used for the code (0..=285; 286/287 never occur).
const NUM_LITLEN: usize = 286;
/// Distance alphabet size (0..=29).
const NUM_DIST: usize = 30;
/// Code-length alphabet size (lengths 0..=15 plus repeat codes 16/17/18).
const NUM_CL: usize = 19;
/// The order in which code-length code lengths are written (RFC 1951 §3.2.7).
const CL_ORDER: [usize; NUM_CL] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// A run-length-coded code-length symbol plus any raw extra bits that follow it.
struct ClItem {
    sym: u16,
    extra_bits: u32,
    extra_val: u32,
}

/// Encodes a parsed LZ77 token stream as a complete dynamic-Huffman DEFLATE body (single block,
/// `BFINAL` set).
pub(crate) fn body(tokens: &[Token]) -> Vec<u8> {
    let mut litlen_hist = [0u32; NUM_LITLEN];
    let mut dist_hist = [0u32; NUM_DIST];
    for &token in tokens {
        match token {
            Token::Literal(b) => litlen_hist[usize::from(b)] += 1,
            Token::Match { len, dist } => {
                litlen_hist[symbols::length_code(len).0 as usize] += 1;
                dist_hist[symbols::distance_code(dist).0 as usize] += 1;
            }
        }
    }
    litlen_hist[256] += 1; // end-of-block is always emitted
    // DEFLATE needs each code complete; forcing >= 2 symbols guarantees that (zlib does the same)
    // and sidesteps the incomplete single-code edge cases in §3.2.7.
    ensure_two(&mut litlen_hist);
    ensure_two(&mut dist_hist);

    let litlen_code = CanonicalCode::from_histogram(&litlen_hist, 15);
    let dist_code = CanonicalCode::from_histogram(&dist_hist, 15);

    let hlit = trimmed_len(litlen_code.lengths(), 257);
    let hdist = trimmed_len(dist_code.lengths(), 1);

    // The lit/len and distance code lengths form a single sequence (RFC 1951 §3.2.7).
    let mut combined = Vec::with_capacity(hlit + hdist);
    combined.extend_from_slice(&litlen_code.lengths()[..hlit]);
    combined.extend_from_slice(&dist_code.lengths()[..hdist]);
    let items = rle(&combined);

    let mut cl_hist = [0u32; NUM_CL];
    for item in &items {
        cl_hist[item.sym as usize] += 1;
    }
    let cl = CanonicalCode::from_histogram(&cl_hist, 7);

    // HCLEN: code-length code lengths in CL_ORDER, with trailing zeros trimmed (minimum of 4).
    let mut hclen = NUM_CL;
    while hclen > 4 && cl.lengths()[CL_ORDER[hclen - 1]] == 0 {
        hclen -= 1;
    }

    let mut w = BitWriter::new();
    w.write_bits(1, 1); // BFINAL = 1
    w.write_bits(0b10, 2); // BTYPE = 10 (dynamic Huffman)
    w.write_bits((hlit - 257) as u32, 5);
    w.write_bits((hdist - 1) as u32, 5);
    w.write_bits((hclen - 4) as u32, 4);
    for &order in CL_ORDER.iter().take(hclen) {
        w.write_bits(u32::from(cl.lengths()[order]), 3);
    }
    for item in &items {
        cl.emit(&mut w, item.sym as usize);
        w.write_bits(item.extra_val, item.extra_bits);
    }
    for &token in tokens {
        match token {
            Token::Literal(b) => litlen_code.emit(&mut w, usize::from(b)),
            Token::Match { len, dist } => {
                let (lsym, lbits, lextra) = symbols::length_code(len);
                litlen_code.emit(&mut w, lsym as usize);
                w.write_bits(lextra, lbits);
                let (dsym, dbits, dextra) = symbols::distance_code(dist);
                dist_code.emit(&mut w, dsym as usize);
                w.write_bits(dextra, dbits);
            }
        }
    }
    litlen_code.emit(&mut w, 256); // end of block
    w.finish()
}

/// Forces a histogram to have at least two used symbols so its Huffman code is complete.
fn ensure_two(hist: &mut [u32]) {
    let used = hist.iter().filter(|&&h| h > 0).count();
    if used >= 2 {
        return;
    }
    if used == 0 {
        hist[0] = 1;
        hist[1] = 1;
    } else {
        let only = hist.iter().position(|&h| h > 0).unwrap_or(0);
        hist[usize::from(only == 0)] = 1;
    }
}

/// Number of leading code lengths to emit: the index past the last nonzero length, but at least
/// `min` (RFC 1951 requires HLIT+257 and HDIST+1 entries, covering every used symbol).
fn trimmed_len(lengths: &[u8], min: usize) -> usize {
    let mut n = lengths.len();
    while n > min && lengths[n - 1] == 0 {
        n -= 1;
    }
    n
}

/// Run-length-codes a code-length sequence into code-length-alphabet symbols (RFC 1951 §3.2.7):
/// 16 = repeat the previous length 3–6 times, 17 = repeat zero 3–10 times, 18 = repeat zero 11–138
/// times. Code 16 only ever repeats within a run of one value, so "previous length" is unambiguous.
fn rle(lengths: &[u8]) -> Vec<ClItem> {
    let mut items = Vec::new();
    let n = lengths.len();
    let mut i = 0;
    while i < n {
        let value = lengths[i];
        let mut j = i + 1;
        while j < n && lengths[j] == value {
            j += 1;
        }
        let mut run = j - i;
        if value == 0 {
            while run >= 11 {
                let count = run.min(138);
                items.push(ClItem {
                    sym: 18,
                    extra_bits: 7,
                    extra_val: (count - 11) as u32,
                });
                run -= count;
            }
            while run >= 3 {
                let count = run.min(10);
                items.push(ClItem {
                    sym: 17,
                    extra_bits: 3,
                    extra_val: (count - 3) as u32,
                });
                run -= count;
            }
            items.extend((0..run).map(|_| ClItem {
                sym: 0,
                extra_bits: 0,
                extra_val: 0,
            }));
        } else {
            // Emit the value literally once, then repeat it with code 16.
            items.push(ClItem {
                sym: u16::from(value),
                extra_bits: 0,
                extra_val: 0,
            });
            run -= 1;
            while run >= 3 {
                let count = run.min(6);
                items.push(ClItem {
                    sym: 16,
                    extra_bits: 2,
                    extra_val: (count - 3) as u32,
                });
                run -= count;
            }
            items.extend((0..run).map(|_| ClItem {
                sym: u16::from(value),
                extra_bits: 0,
                extra_val: 0,
            }));
        }
        i = j;
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Expands a code-length RLE back to the flat length sequence it encodes.
    fn expand(items: &[ClItem]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut prev = 0u8;
        for item in items {
            match item.sym {
                0..=15 => {
                    out.push(item.sym as u8);
                    prev = item.sym as u8;
                }
                16 => out.resize(out.len() + (3 + item.extra_val) as usize, prev),
                17 => out.resize(out.len() + (3 + item.extra_val) as usize, 0),
                18 => out.resize(out.len() + (11 + item.extra_val) as usize, 0),
                _ => panic!("unexpected code-length symbol {}", item.sym),
            }
        }
        out
    }

    #[test]
    fn rle_round_trips_various_runs() {
        let cases: &[Vec<u8>] = &[
            vec![],
            vec![5],
            vec![0; 200],                 // long zero run -> 18s
            vec![7; 100],                 // long nonzero run -> literal + 16s
            vec![0, 0, 0, 8, 8, 8, 8, 0], // mixed
            vec![3, 0, 0, 0, 0, 0, 6, 6], // short zero run -> 17
            (0..40u8).map(|i| i % 16).collect(),
        ];
        for case in cases {
            assert_eq!(&expand(&rle(case)), case, "RLE failed for {case:?}");
        }
    }

    #[test]
    fn trimmed_len_keeps_minimum() {
        assert_eq!(trimmed_len(&[1, 1, 0, 0], 1), 2);
        assert_eq!(trimmed_len(&[0, 0, 0, 0], 1), 1); // never below the minimum
        assert_eq!(trimmed_len(&[1, 0, 1, 0], 1), 3); // stops past the last nonzero
    }

    #[test]
    fn ensure_two_adds_symbols() {
        let mut none = [0u32; 5];
        ensure_two(&mut none);
        assert_eq!(none.iter().filter(|&&h| h > 0).count(), 2);
        let mut one = [0u32, 9, 0];
        ensure_two(&mut one);
        assert_eq!(one.iter().filter(|&&h| h > 0).count(), 2);
    }
}
