//! Strict "deconstruct" decoding: walk the whole DNG and prove every byte was accounted for.
//!
//! This is the archival counterpart to [`crate::decoder`], filed as issue #197 from the PR #161
//! review. Ordinary decoding reads the raw image, profile, and metadata it needs and ignores the
//! rest. [`deconstruct`] instead walks the entire IFD tree — IFD 0 and its chain, the `SubIFDs`
//! image sub-IFDs (including the full-resolution raw), and the Exif/GPS metadata sub-IFDs — marking
//! every consumed byte into a [`gamut_ifd::Coverage`] and layering DNG tag knowledge on top. The
//! resulting [`DeconstructReport`] surfaces unaccounted bytes (gaps / trailing / overlaps), unknown
//! field types, unknown/private tags, out-of-spec codes, and malformed colour matrices instead of
//! silently dropping them.
//!
//! It is **collect-and-report, not fail-fast**: it errors only when the container itself is
//! unreadable, exactly as [`gamut_ifd::read`] would; everything else is reported for the caller to
//! judge.

use gamut_core::Result;
use gamut_ifd::{
    ByteOrder, Coverage, CoverageReport, Ifd, UnknownField, Value, Variant,
    read_ifd_at_with_coverage, read_with_coverage,
};

use crate::decoder::{DecodedDng, DngDecoder};
use crate::tags;
use crate::values::{Compression, PhotometricInterpretation};

/// An upper bound on sub-IFD nesting, guarding the recursive walk against malformed/looping trees.
const MAX_SUBIFD_DEPTH: usize = 16;

/// How serious a reported [`Anomaly`] is.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Out of spec but often benign.
    Warning,
    /// A structural defect (e.g. a strip mismatch, a malformed matrix, or a sub-IFD cycle).
    Error,
}

/// A tag a valid DNG would not be expected to carry — recognised structurally but not part of the
/// DNG 1.7.1 tag set (see [`tags::is_known_tag`]).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownTag {
    /// The page (top-level IFD index) the tag was found in.
    pub page: usize,
    /// The tag number.
    pub tag: u16,
    /// The tag's on-disk field-type code.
    pub field_type: u16,
    /// The tag's value count.
    pub count: u64,
}

/// A recognised but out-of-spec or unparsable element a deconstruct flags.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Anomaly {
    /// A tag whose value uses a code this crate does not recognise (e.g. an unknown `Compression`).
    UnknownCode {
        /// The page the tag was found in.
        page: usize,
        /// The tag carrying the code.
        tag: u16,
        /// The unrecognised code.
        code: u32,
        /// A human-readable description.
        detail: &'static str,
    },
    /// A known tag whose value could not be interpreted (wrong type, count, or range).
    UnparsableTag {
        /// The page the tag was found in.
        page: usize,
        /// The tag.
        tag: u16,
        /// A human-readable description.
        detail: &'static str,
    },
    /// An out-of-spec or unexpected structural condition.
    Structure {
        /// The page the condition relates to.
        page: usize,
        /// A human-readable description.
        detail: &'static str,
        /// How serious the condition is.
        severity: Severity,
    },
}

/// The result of a strict deconstruct: byte-range accounting plus DNG-specific findings.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct DeconstructReport {
    /// Which file bytes were accounted for, and the gaps/overlaps/trailing that were not.
    pub coverage: CoverageReport,
    /// IFD entries whose field-type code was unrecognised (and thus skipped by the reader).
    pub unknown_fields: Vec<UnknownField>,
    /// Tags not part of the recognised DNG tag set.
    pub unknown_tags: Vec<UnknownTag>,
    /// Recognised-but-out-of-spec codes, unparsable tags, and structural problems.
    pub anomalies: Vec<Anomaly>,
}

impl DeconstructReport {
    /// Whether every byte of the file was accounted for exactly once (delegates to
    /// [`CoverageReport::is_fully_covered`]). Independent of whether every tag/code was recognised.
    #[must_use]
    pub fn is_fully_covered(&self) -> bool {
        self.coverage.is_fully_covered()
    }

    /// Whether the file is pristine for archival: fully covered, with no unknown field types, no
    /// unknown tags, and no anomalies.
    #[must_use]
    pub fn is_fully_accounted(&self) -> bool {
        self.coverage.is_fully_covered()
            && self.unknown_fields.is_empty()
            && self.unknown_tags.is_empty()
            && self.anomalies.is_empty()
    }
}

/// Walks `data` (a DNG file) and returns a [`DeconstructReport`] accounting every byte — without
/// decoding the raw image.
///
/// # Errors
///
/// Returns [`gamut_core::Error::InvalidInput`] only when the container is unreadable (bad header,
/// truncated/looping IFD chain) — the same conditions under which [`gamut_ifd::read`] fails.
/// Unknown tags, unknown field types, out-of-spec codes, malformed matrices, and unaccounted bytes
/// are reported, not errored.
pub fn deconstruct(data: &[u8]) -> Result<DeconstructReport> {
    let mut cov = Coverage::new(data.len() as u64);
    let mut unknown_fields = Vec::new();
    let file = read_with_coverage(data, &mut cov, &mut unknown_fields)?;
    let mut d = Deconstructor {
        data,
        order: file.order,
        variant: file.variant,
        cov,
        unknown_fields,
        unknown_tags: Vec::new(),
        anomalies: Vec::new(),
        visited: Vec::new(),
    };
    for (page, ifd) in file.ifds.iter().enumerate() {
        d.account_image_ifd(ifd, page, 0);
    }
    let coverage = d.cov.finish();
    Ok(DeconstructReport {
        coverage,
        unknown_fields: d.unknown_fields,
        unknown_tags: d.unknown_tags,
        anomalies: d.anomalies,
    })
}

/// Accumulates the byte-range accounting and DNG findings while walking the IFD tree.
struct Deconstructor<'a> {
    data: &'a [u8],
    order: ByteOrder,
    variant: Variant,
    cov: Coverage,
    unknown_fields: Vec<UnknownField>,
    unknown_tags: Vec<UnknownTag>,
    anomalies: Vec<Anomaly>,
    /// Sub-IFD offsets already visited, to break cycles across the whole tree.
    visited: Vec<u64>,
}

impl Deconstructor<'_> {
    /// Accounts an image IFD (IFD 0, the raw sub-IFD, or a preview/image sub-IFD): its tags, codes,
    /// colour matrices, pixel data, and the image / metadata sub-IFDs it points at.
    fn account_image_ifd(&mut self, ifd: &Ifd, page: usize, depth: usize) {
        self.check_tags(ifd, page);
        self.check_codes(ifd, page);
        self.check_matrices(ifd, page);
        self.account_pixels(ifd, page);
        self.follow_image_subifds(ifd, page, depth);
        self.follow_metadata(ifd, page, depth);
    }

    /// Flags every field whose tag is not part of the recognised DNG tag set.
    fn check_tags(&mut self, ifd: &Ifd, page: usize) {
        for field in ifd.fields() {
            if !tags::is_known_tag(field.tag) {
                self.unknown_tags.push(UnknownTag {
                    page,
                    tag: field.tag,
                    field_type: field.value.field_type().code(),
                    count: field.value.count() as u64,
                });
            }
        }
    }

    /// Flags `Compression` and `PhotometricInterpretation` values whose code this crate does not
    /// recognise.
    fn check_codes(&mut self, ifd: &Ifd, page: usize) {
        if let Some(code) = ifd.get_u32(tags::COMPRESSION)
            && u16::try_from(code)
                .ok()
                .and_then(Compression::from_code)
                .is_none()
        {
            self.anomalies.push(Anomaly::UnknownCode {
                page,
                tag: tags::COMPRESSION,
                code,
                detail: "DNG: unrecognised Compression code",
            });
        }
        if let Some(code) = ifd.get_u32(tags::PHOTOMETRIC_INTERPRETATION)
            && u16::try_from(code)
                .ok()
                .and_then(PhotometricInterpretation::from_code)
                .is_none()
        {
            self.anomalies.push(Anomaly::UnknownCode {
                page,
                tag: tags::PHOTOMETRIC_INTERPRETATION,
                code,
                detail: "DNG: unrecognised PhotometricInterpretation code",
            });
        }
    }

    /// Flags 3×3 colour-calibration matrix tags that are present but not nine `(S)RATIONAL`s.
    fn check_matrices(&mut self, ifd: &Ifd, page: usize) {
        for &tag in tags::MATRIX_3X3_TAGS {
            let Some(value) = ifd.get(tag) else {
                continue;
            };
            let ok = matches!(value, Value::Rational(v) if v.len() == 9)
                || matches!(value, Value::SRational(v) if v.len() == 9);
            if !ok {
                self.anomalies.push(Anomaly::UnparsableTag {
                    page,
                    tag,
                    detail: "DNG: 3x3 matrix tag is not nine rationals",
                });
            }
        }
    }

    /// Marks the byte ranges of an image IFD's strips or tiles.
    fn account_pixels(&mut self, ifd: &Ifd, page: usize) {
        if ifd.get(tags::TILE_WIDTH).is_some() || ifd.get(tags::TILE_OFFSETS).is_some() {
            self.account_ranges(
                ifd,
                page,
                tags::TILE_OFFSETS,
                tags::TILE_BYTE_COUNTS,
                "DNG: TileOffsets/TileByteCounts length mismatch",
                "DNG: tiled image missing TileOffsets/TileByteCounts",
            );
        } else if ifd.get(tags::STRIP_OFFSETS).is_some() {
            self.account_ranges(
                ifd,
                page,
                tags::STRIP_OFFSETS,
                tags::STRIP_BYTE_COUNTS,
                "DNG: StripOffsets/StripByteCounts length mismatch",
                "DNG: image missing StripOffsets/StripByteCounts",
            );
        } else if ifd.get_u32(tags::IMAGE_WIDTH).is_some() {
            self.anomalies.push(Anomaly::Structure {
                page,
                detail: "DNG: image IFD has no strip or tile data",
                severity: Severity::Warning,
            });
        }
    }

    /// Marks each `(offset, byte_count)` pair of a strip/tile array, flagging length mismatches.
    fn account_ranges(
        &mut self,
        ifd: &Ifd,
        page: usize,
        off_tag: u16,
        cnt_tag: u16,
        mismatch: &'static str,
        missing: &'static str,
    ) {
        let (Some(offsets), Some(counts)) = (ifd.get_u32_vec(off_tag), ifd.get_u32_vec(cnt_tag))
        else {
            self.anomalies.push(Anomaly::Structure {
                page,
                detail: missing,
                severity: Severity::Error,
            });
            return;
        };
        if offsets.len() != counts.len() {
            self.anomalies.push(Anomaly::Structure {
                page,
                detail: mismatch,
                severity: Severity::Error,
            });
        }
        for (&off, &cnt) in offsets.iter().zip(&counts) {
            self.cov.mark(u64::from(off), u64::from(cnt));
        }
    }

    /// Follows the `SubIFDs` (330) pointers, accounting each child as another image IFD (this is how
    /// the full-resolution raw sub-IFD is reached).
    fn follow_image_subifds(&mut self, ifd: &Ifd, page: usize, depth: usize) {
        let Some(offsets) = ifd.get_u32_vec(tags::SUB_IFDS) else {
            return;
        };
        for off in offsets {
            let off = u64::from(off);
            if !self.guard(off, page, depth + 1) {
                continue;
            }
            if let Some(sub) = self.read_sub(off, page) {
                self.account_image_ifd(&sub, page, depth + 1);
            }
        }
    }

    /// Follows the Exif (34665) and GPS (34853) sub-IFD pointers, accounting their bytes (their tags
    /// belong to a separate namespace, so they are not tag-checked here).
    fn follow_metadata(&mut self, ifd: &Ifd, page: usize, depth: usize) {
        for tag in [tags::EXIF_IFD, tags::GPS_INFO] {
            if let Some(off) = ifd.get_u32(tag) {
                self.account_meta_tree(u64::from(off), page, depth + 1);
            }
        }
    }

    /// Accounts a metadata sub-IFD and any nested Interoperability sub-IFD (40965).
    fn account_meta_tree(&mut self, offset: u64, page: usize, depth: usize) {
        if !self.guard(offset, page, depth) {
            return;
        }
        let Some(sub) = self.read_sub(offset, page) else {
            return;
        };
        // The Exif Interoperability pointer (40965) is the one nested metadata sub-IFD DNG uses.
        if let Some(off) = sub.get_u32(40965) {
            self.account_meta_tree(u64::from(off), page, depth + 1);
        }
    }

    /// Parses (and accounts the bytes of) a sub-IFD at `offset`, recording an anomaly if it cannot
    /// be parsed.
    fn read_sub(&mut self, offset: u64, page: usize) -> Option<Ifd> {
        let (data, order, variant) = (self.data, self.order, self.variant);
        match read_ifd_at_with_coverage(
            data,
            offset,
            order,
            variant,
            &mut self.cov,
            &mut self.unknown_fields,
        ) {
            Ok((ifd, _next)) => Some(ifd),
            Err(_) => {
                self.anomalies.push(Anomaly::Structure {
                    page,
                    detail: "DNG: sub-IFD could not be parsed",
                    severity: Severity::Error,
                });
                None
            }
        }
    }

    /// Returns whether a sub-IFD at `offset` may be followed, recording an anomaly (and refusing)
    /// on excessive depth or a revisited offset (cycle).
    fn guard(&mut self, offset: u64, page: usize, depth: usize) -> bool {
        if depth > MAX_SUBIFD_DEPTH {
            self.anomalies.push(Anomaly::Structure {
                page,
                detail: "DNG: sub-IFD nesting too deep",
                severity: Severity::Error,
            });
            return false;
        }
        if self.visited.contains(&offset) {
            self.anomalies.push(Anomaly::Structure {
                page,
                detail: "DNG: sub-IFD offset revisited (possible cycle)",
                severity: Severity::Error,
            });
            return false;
        }
        self.visited.push(offset);
        true
    }
}

impl DngDecoder {
    /// Decodes `data` to its [`DecodedDng`] *and* returns a whole-file [`DeconstructReport`] that
    /// accounts every byte and flags anything unrecognised.
    ///
    /// # Errors
    ///
    /// Returns [`gamut_core::Error`] if the container is unreadable or the raw image cannot be
    /// decoded (e.g. an out-of-scope compression) — a strict superset of [`DngDecoder::decode`].
    /// Use the free [`deconstruct`] function for a report without decoding.
    pub fn deconstruct(&self, data: &[u8]) -> Result<(DecodedDng, DeconstructReport)> {
        let report = deconstruct(data)?;
        let decoded = self.decode(data)?;
        Ok((decoded, report))
    }
}
