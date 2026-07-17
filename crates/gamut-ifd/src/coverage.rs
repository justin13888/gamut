//! Byte-range accounting over a TIFF/IFD stream, for strict "deconstruct" decoding.
//!
//! Ordinary decoding reads the structures it needs and ignores the rest. For archival / critical
//! use a stricter guarantee is wanted: that **every byte of the file was accounted for**, with
//! anything left over (or claimed twice, or reaching past the end) surfaced rather than silently
//! dropped. [`Coverage`] is the shared engine for that — a decoder [`mark`](Coverage::mark)s each
//! byte range it consumes (the header, each IFD body, each out-of-line value, each strip/tile),
//! then [`finish`](Coverage::finish)es into a [`CoverageReport`] of the gaps, overlaps, trailing
//! bytes, and any out-of-bounds claims.
//!
//! The format codec drives this: it owns one `Coverage` and threads it through the
//! coverage-recording reader entry points ([`read_with_coverage`](crate::read_with_coverage),
//! [`read_ifd_at_with_coverage`](crate::read_ifd_at_with_coverage)) and its own strip/tile
//! reads, then layers format-specific tag knowledge (known vs unknown tags, out-of-spec codes)
//! on top of the structural report this produces.

use crate::segment::Range;

/// Two marked ranges that share at least one byte — a structure claiming bytes another already
/// claimed, which an archival validator treats as out-of-spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Overlap {
    /// The range already covered (the merge of everything claimed so far in this region).
    pub a: Range,
    /// The newly marked range that overlapped it.
    pub b: Range,
}

/// An IFD entry whose on-disk field-type code is not recognised.
///
/// The readers preserve such entries verbatim as [`Value::Unknown`](crate::Value::Unknown); the
/// coverage path additionally records them here so a deconstruct can report them with their
/// file positions. The 12-/20-byte entry record itself is covered by the enclosing IFD-body
/// mark, but the entry's out-of-line value bytes (if any) cannot be sized — so they surface as
/// a coverage gap, which is the correct archival signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownField {
    /// The file offset of the IFD that held the entry.
    pub ifd_offset: u64,
    /// The entry's tag.
    pub tag: u16,
    /// The unrecognised field-type code.
    pub type_code: u16,
    /// The entry's declared value count (reported as-is; it may be untrustworthy).
    pub count: u64,
    /// The file offset of the 12-/20-byte entry record.
    pub entry_offset: u64,
}

/// An accumulator of the byte ranges a decoder consumed while walking a file.
///
/// Ranges are appended in parse order via [`mark`](Self::mark) and only sorted/merged once, in
/// [`finish`](Self::finish) — there is no online query during parsing, so a plain `Vec` with a
/// single terminal merge pass (`O(n log n)`) is used rather than an interval tree. The internal
/// representation is private, so it can change without breaking callers.
#[derive(Debug, Clone)]
pub struct Coverage {
    file_len: u64,
    ranges: Vec<Range>,
    out_of_bounds: Vec<Range>,
}

impl Coverage {
    /// Creates an accumulator for a file of `file_len` bytes.
    #[must_use]
    pub fn new(file_len: u64) -> Self {
        Self {
            file_len,
            ranges: Vec::new(),
            out_of_bounds: Vec::new(),
        }
    }

    /// Records that the range `[start, start + len)` was consumed.
    ///
    /// A zero-length mark is ignored. A range that reaches past the end of the file is recorded in
    /// the report's [`out_of_bounds`](CoverageReport::out_of_bounds) list (as supplied), and its
    /// in-bounds portion, if any, still counts toward coverage.
    pub fn mark(&mut self, start: u64, len: u64) {
        if len == 0 {
            return;
        }
        let end = start.saturating_add(len);
        if end > self.file_len {
            // Record the offending range as supplied for diagnostics, and clamp the in-bounds part
            // so the bytes that *were* valid still count toward coverage.
            self.out_of_bounds.push(Range { start, len });
            if start < self.file_len {
                self.ranges.push(Range {
                    start,
                    len: self.file_len - start,
                });
            }
        } else {
            self.ranges.push(Range { start, len });
        }
    }

    /// Sorts and merges the marked ranges into a [`CoverageReport`] of gaps, overlaps, trailing
    /// bytes, and the total covered byte count.
    #[must_use]
    pub fn finish(mut self) -> CoverageReport {
        self.ranges
            .sort_by(|a, b| a.start.cmp(&b.start).then(a.end().cmp(&b.end())));
        let mut merged: Vec<Range> = Vec::new();
        let mut overlaps: Vec<Overlap> = Vec::new();
        for r in &self.ranges {
            if let Some(last) = merged.last_mut() {
                if r.start < last.end() {
                    // Overlap: the new range claims bytes already covered.
                    overlaps.push(Overlap { a: *last, b: *r });
                    let new_end = last.end().max(r.end());
                    last.len = new_end - last.start;
                    continue;
                }
                if r.start == last.end() {
                    // Adjacent: extend without flagging an overlap.
                    last.len = r.end() - last.start;
                    continue;
                }
            }
            merged.push(*r);
        }
        let covered_bytes = merged.iter().map(|r| r.len).sum();
        let mut gaps = Vec::new();
        let mut cursor = 0u64;
        for r in &merged {
            if r.start > cursor {
                gaps.push(Range {
                    start: cursor,
                    len: r.start - cursor,
                });
            }
            cursor = r.end();
        }
        let trailing = (cursor < self.file_len).then_some(Range {
            start: cursor,
            len: self.file_len - cursor,
        });
        CoverageReport {
            file_len: self.file_len,
            gaps,
            trailing,
            overlaps,
            out_of_bounds: self.out_of_bounds,
            covered_bytes,
        }
    }
}

/// The result of accounting a file's bytes: what was covered and what was not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageReport {
    /// The total size of the file, in bytes.
    pub file_len: u64,
    /// Interior unaccounted ranges (bytes between two covered regions).
    pub gaps: Vec<Range>,
    /// The final unaccounted range reaching the end of the file, if any (e.g. trailing padding).
    pub trailing: Option<Range>,
    /// Ranges claimed by two different structures.
    pub overlaps: Vec<Overlap>,
    /// Marked ranges that reached past the end of the file.
    pub out_of_bounds: Vec<Range>,
    /// The number of distinct in-bounds bytes covered (overlaps counted once).
    pub covered_bytes: u64,
}

impl CoverageReport {
    /// Whether every byte of the file was accounted for exactly once: no gaps, no trailing bytes,
    /// no overlaps, and nothing out of bounds.
    #[must_use]
    pub fn is_fully_covered(&self) -> bool {
        self.gaps.is_empty()
            && self.trailing.is_none()
            && self.overlaps.is_empty()
            && self.out_of_bounds.is_empty()
    }

    /// The number of bytes not covered (`file_len - covered_bytes`).
    #[must_use]
    pub fn unaccounted_bytes(&self) -> u64 {
        self.file_len.saturating_sub(self.covered_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(file_len: u64, marks: &[(u64, u64)]) -> CoverageReport {
        let mut cov = Coverage::new(file_len);
        for &(start, len) in marks {
            cov.mark(start, len);
        }
        cov.finish()
    }

    #[test]
    fn full_contiguous_coverage_is_clean() {
        let r = report(10, &[(0, 4), (4, 6)]);
        assert!(r.is_fully_covered());
        assert_eq!(r.covered_bytes, 10);
        assert_eq!(r.unaccounted_bytes(), 0);
        assert!(r.gaps.is_empty() && r.trailing.is_none() && r.overlaps.is_empty());
    }

    #[test]
    fn out_of_order_marks_merge() {
        // Adjacency must hold regardless of insertion order; finish sorts first.
        let r = report(10, &[(4, 6), (0, 4)]);
        assert!(r.is_fully_covered());
    }

    #[test]
    fn interior_gap_is_reported_not_trailing() {
        let r = report(20, &[(0, 5), (10, 10)]);
        assert_eq!(r.gaps, vec![Range { start: 5, len: 5 }]);
        assert_eq!(r.trailing, None);
        assert!(!r.is_fully_covered());
        assert_eq!(r.covered_bytes, 15);
    }

    #[test]
    fn trailing_is_separate_from_gaps() {
        let r = report(20, &[(0, 12)]);
        assert!(r.gaps.is_empty());
        assert_eq!(r.trailing, Some(Range { start: 12, len: 8 }));
        assert!(!r.is_fully_covered());
    }

    #[test]
    fn leading_gap_is_an_interior_gap() {
        let r = report(10, &[(3, 7)]);
        assert_eq!(r.gaps, vec![Range { start: 0, len: 3 }]);
        assert_eq!(r.trailing, None);
    }

    #[test]
    fn overlap_is_detected_and_covered_once() {
        let r = report(10, &[(0, 6), (4, 6)]);
        assert_eq!(r.overlaps.len(), 1);
        assert_eq!(r.overlaps[0].a, Range { start: 0, len: 6 });
        assert_eq!(r.overlaps[0].b, Range { start: 4, len: 6 });
        // Overlap never inflates the covered count past the union.
        assert_eq!(r.covered_bytes, 10);
        assert!(!r.is_fully_covered());
    }

    #[test]
    fn nested_range_is_overlap_without_extending() {
        let r = report(10, &[(0, 10), (2, 3)]);
        assert_eq!(r.overlaps.len(), 1);
        assert_eq!(r.covered_bytes, 10);
        assert!(r.trailing.is_none() && r.gaps.is_empty());
    }

    #[test]
    fn identical_ranges_overlap() {
        let r = report(8, &[(0, 8), (0, 8)]);
        assert_eq!(r.overlaps.len(), 1);
        assert_eq!(r.covered_bytes, 8);
    }

    #[test]
    fn zero_length_marks_are_ignored() {
        let r = report(10, &[(0, 10), (5, 0)]);
        assert!(r.is_fully_covered());
    }

    #[test]
    fn out_of_bounds_is_recorded_and_clamped() {
        let r = report(10, &[(0, 5), (8, 6)]);
        assert_eq!(r.out_of_bounds, vec![Range { start: 8, len: 6 }]);
        // The in-bounds portion [8,10) still counts.
        assert_eq!(r.covered_bytes, 7);
        assert!(!r.is_fully_covered());
    }

    /// `start == file_len` is the mark-clamp boundary: nothing is in bounds, so the report shows
    /// the whole file as trailing — not a phantom zero-length range that would misreport the
    /// uncovered span as an interior gap.
    #[test]
    fn mark_at_exactly_file_end_is_out_of_bounds_only() {
        let r = report(10, &[(10, 4)]);
        assert_eq!(r.out_of_bounds, vec![Range { start: 10, len: 4 }]);
        assert_eq!(r.covered_bytes, 0);
        assert!(r.gaps.is_empty());
        assert_eq!(r.trailing, Some(Range { start: 0, len: 10 }));
    }

    /// Overlap and adjacency merges away from offset 0, with exact covered/gap/trailing spans —
    /// `new_end - last.start` and `new_end + last.start` coincide when `last.start == 0`, so a
    /// non-zero start is what actually pins the merge arithmetic.
    #[test]
    fn merges_away_from_origin_have_exact_extents() {
        let r = report(20, &[(5, 6), (8, 6)]); // overlap on [8, 11)
        assert_eq!(r.overlaps.len(), 1);
        assert_eq!(r.covered_bytes, 9); // [5, 14)
        assert_eq!(r.gaps, vec![Range { start: 0, len: 5 }]);
        assert_eq!(r.trailing, Some(Range { start: 14, len: 6 }));

        let r = report(20, &[(5, 5), (10, 5)]); // adjacent at 10
        assert!(r.overlaps.is_empty());
        assert_eq!(r.covered_bytes, 10); // [5, 15)
        assert_eq!(r.trailing, Some(Range { start: 15, len: 5 }));
    }

    #[test]
    fn fully_out_of_bounds_mark_covers_nothing() {
        let r = report(10, &[(0, 10), (20, 4)]);
        assert_eq!(r.out_of_bounds, vec![Range { start: 20, len: 4 }]);
        assert_eq!(r.covered_bytes, 10);
        assert!(!r.is_fully_covered());
    }

    #[test]
    fn empty_coverage_is_all_trailing() {
        let r = report(10, &[]);
        assert_eq!(r.trailing, Some(Range { start: 0, len: 10 }));
        assert_eq!(r.covered_bytes, 0);
        assert_eq!(r.unaccounted_bytes(), 10);
    }

    #[test]
    fn single_range_equal_to_file_is_clean() {
        let r = report(42, &[(0, 42)]);
        assert!(r.is_fully_covered());
        assert_eq!(r.covered_bytes, 42);
    }
}
