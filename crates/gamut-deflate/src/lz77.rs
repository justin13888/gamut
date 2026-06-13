//! LZ77 parsing: a chained-hash longest-match finder over the input bytes (RFC 1951 §4).
//!
//! The parser turns the byte stream into a sequence of [`Token`]s — literals and `(length,
//! distance)` back-references — which a block writer then entropy-codes. Match *correctness* never
//! depends on the hash; candidates are always verified by byte comparison. The hash only affects how
//! many real matches are found, i.e. the ratio.
//!
//! This phase emits a greedy parse (take the longest match at each position). Lazy matching and the
//! zopfli-style optimal parse build on the same matcher in later phases.

use crate::symbols::{MAX_DISTANCE, MAX_MATCH, MIN_MATCH};

/// One element of the LZ77 token stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Token {
    /// A single uncompressed byte.
    Literal(u8),
    /// A back-reference: copy `len` bytes (3..=258) from `dist` (1..=32768) bytes earlier.
    Match { len: u16, dist: u16 },
}

/// Number of hash buckets is `1 << HASH_BITS`.
const HASH_BITS: u32 = 15;
/// Number of hash buckets.
const HASH_SIZE: usize = 1 << HASH_BITS;
/// The sliding window is the maximum back-reference distance; `prev` is indexed modulo it.
const WINDOW: usize = MAX_DISTANCE;
/// Mask for the power-of-two window.
const WMASK: usize = WINDOW - 1;

/// A chained-hash index over already-seen 3-byte sequences, used to find back-references.
///
/// `head[h]` is the most recent position whose 3-byte hash is `h`; `prev[pos & WMASK]` chains to the
/// next-older position in the same bucket. Both store absolute positions (`-1` = empty). Entries
/// older than the window are pruned by the distance check during search, so the modular `prev`
/// indexing never returns a stale match.
struct Matcher {
    head: Vec<i32>,
    prev: Vec<i32>,
}

impl Matcher {
    fn new() -> Self {
        Self {
            head: vec![-1; HASH_SIZE],
            prev: vec![-1; WINDOW],
        }
    }

    /// Hashes the 3 bytes at `pos` (caller guarantees `pos + 3 <= data.len()`).
    fn hash(data: &[u8], pos: usize) -> usize {
        let key = (u32::from(data[pos]) << 16)
            | (u32::from(data[pos + 1]) << 8)
            | u32::from(data[pos + 2]);
        (key.wrapping_mul(0x9E37_79B1) >> (32 - HASH_BITS)) as usize & (HASH_SIZE - 1)
    }

    /// Records `pos` as a future match candidate.
    fn insert(&mut self, data: &[u8], pos: usize) {
        if pos + MIN_MATCH > data.len() {
            return;
        }
        let h = Self::hash(data, pos);
        self.prev[pos & WMASK] = self.head[h];
        self.head[h] = pos as i32;
    }

    /// Finds the longest back-reference for the bytes at `pos`, walking at most `max_chain`
    /// candidates. Returns `(len, dist)` with `len >= MIN_MATCH` and `1 <= dist <= 32768`.
    fn find(&self, data: &[u8], pos: usize, max_chain: usize) -> Option<(u16, u16)> {
        let n = data.len();
        if pos + MIN_MATCH > n {
            return None;
        }
        let max_len = (n - pos).min(MAX_MATCH);
        let lowest = pos.saturating_sub(WINDOW); // candidates must be >= this for dist <= window
        let mut best_len = MIN_MATCH - 1; // a real match must strictly exceed this
        let mut best_dist = 0usize;
        let mut cand = self.head[Self::hash(data, pos)];
        let mut chain = 0usize;
        while cand >= 0 {
            let c = cand as usize;
            if c < lowest {
                break; // out of window; chain only gets older from here
            }
            // Prune: a candidate can only win if it matches at the byte just past the current best.
            if best_len < max_len && data[c + best_len] == data[pos + best_len] {
                let mut len = 0usize;
                while len < max_len && data[c + len] == data[pos + len] {
                    len += 1;
                }
                if len > best_len {
                    best_len = len;
                    best_dist = pos - c;
                    if len >= max_len {
                        break; // can't do better than the maximum
                    }
                }
            }
            chain += 1;
            if chain >= max_chain {
                break;
            }
            cand = self.prev[c & WMASK];
        }
        if best_len >= MIN_MATCH {
            Some((best_len as u16, best_dist as u16))
        } else {
            None
        }
    }
}

/// Parses `data` into an LZ77 token stream, searching up to `max_chain` candidates per position.
///
/// With `lazy` set, the parser uses lazy matching (RFC 1951 §4): after finding a match it checks
/// whether the next position starts a longer one and, if so, defers — emitting the current byte as a
/// literal. This finds better parses than pure greedy at a small time cost. A larger `max_chain`
/// finds more/longer matches, also at a time cost.
pub(crate) fn parse(data: &[u8], max_chain: usize, lazy: bool) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut matcher = Matcher::new();
    let n = data.len();
    let mut pos = 0;
    while pos < n {
        let current = matcher.find(data, pos, max_chain);
        matcher.insert(data, pos); // pos becomes a candidate for subsequent positions
        let Some((len, dist)) = current else {
            tokens.push(Token::Literal(data[pos]));
            pos += 1;
            continue;
        };
        // Lazy matching: if the next position begins a strictly longer match, defer this one.
        if lazy
            && (len as usize) < MAX_MATCH
            && pos + 1 < n
            && let Some((next_len, _)) = matcher.find(data, pos + 1, max_chain)
            && next_len > len
        {
            tokens.push(Token::Literal(data[pos]));
            pos += 1;
            continue;
        }
        tokens.push(Token::Match { len, dist });
        // `pos` is already inserted; insert the rest of the covered span so future matches can
        // reference inside it.
        let end = pos + len as usize;
        for p in (pos + 1)..end {
            matcher.insert(data, p);
        }
        pos = end;
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_literals_when_no_repeats() {
        let data = [1u8, 2, 3, 4, 5];
        let tokens = parse(&data, 128, false);
        assert_eq!(tokens.len(), 5);
        assert!(tokens.iter().all(|t| matches!(t, Token::Literal(_))));
    }

    #[test]
    fn finds_a_repeated_block() {
        // "abcabc": positions 0-2 are literals, then a match (len 3, dist 3).
        let data = b"abcabc";
        let tokens = parse(data, 128, false);
        assert_eq!(tokens[0], Token::Literal(b'a'));
        assert_eq!(tokens[1], Token::Literal(b'b'));
        assert_eq!(tokens[2], Token::Literal(b'c'));
        assert_eq!(tokens[3], Token::Match { len: 3, dist: 3 });
    }

    #[test]
    fn long_run_uses_overlapping_match() {
        // A run of identical bytes becomes a literal then a long overlapping match at distance 1.
        let data = vec![0x5Au8; 300];
        let tokens = parse(&data, 128, false);
        assert_eq!(tokens[0], Token::Literal(0x5A));
        // The next token copies at distance 1, capped at the 258-byte maximum length.
        assert_eq!(tokens[1], Token::Match { len: 258, dist: 1 });
    }

    #[test]
    fn respects_minimum_match_length() {
        // A 2-byte repeat is too short to reference; it stays literal.
        let data = b"abxxab";
        let tokens = parse(data, 128, false);
        assert!(tokens.iter().all(|t| matches!(t, Token::Literal(_))));
    }
}
