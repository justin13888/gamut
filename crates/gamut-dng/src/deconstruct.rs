//! Strict "deconstruct" decoding: walk the whole DNG and prove every byte is classified.
//!
//! This is the archival counterpart to [`crate::decoder`], filed as issue #197 from the PR #161
//! review and rebuilt on the shared structural auditor for issue #263. Ordinary decoding reads
//! the raw image, profile, and metadata it needs and ignores the rest. [`deconstruct`] instead
//! drives [`gamut_ifd::audit`] over the entire IFD tree — IFD 0 and its chain, the `SubIFDs`
//! image sub-IFDs (including the full-resolution raw), the Exif/GPS/Interoperability metadata
//! sub-IFDs, and every strip/tile/free/embedded-JPEG extent — then layers DNG tag knowledge on
//! top. The resulting [`DeconstructReport`] carries the byte-level [`SegmentReport`] (every byte
//! typed, or precisely what was not — including the dual-ledger parser cross-check) plus unknown
//! field types, unknown/private tags, out-of-spec codes, and malformed colour matrices.
//!
//! It is **collect-and-report, not fail-fast**: it errors only when the container itself is
//! unreadable, exactly as [`gamut_ifd::read`] would; everything else is reported for the caller
//! to judge.

use gamut_core::Result;
use gamut_ifd::{
    AuditFinding, Ifd, SegmentReport, SkipReason, StandardAuditSpec, Value, audit as ifd_audit,
};

use crate::decoder::{DecodedDng, DngDecoder};
use crate::tags;
use crate::values::{Compression, PhotometricInterpretation};

/// How serious a reported [`Anomaly`] is.
///
/// Non-exhaustive: further severities may be added without a breaking change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Severity {
    /// Out of spec but often benign.
    Warning,
    /// A structural defect (e.g. a strip mismatch, a malformed matrix, or a sub-IFD cycle).
    Error,
}

/// A tag a valid DNG would not be expected to carry — recognised structurally but not part of the
/// DNG 1.7.1 tag set (see [`tags::is_known_tag`]).
///
/// Non-exhaustive: fields may be added as the deconstruct grows more precise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
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

/// An IFD entry whose field-type code is unrecognised. The entry is **preserved verbatim** in
/// the parse (as a [`gamut_ifd::Value::Unknown`]) — reported here because its out-of-line
/// payload, if any, is unsizable and therefore unclassifiable.
///
/// Non-exhaustive: fields may be added as the deconstruct grows more precise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct UnknownFieldType {
    /// The page (top-level IFD index) the entry was found in.
    pub page: usize,
    /// The entry's tag.
    pub tag: u16,
    /// The unrecognised on-disk field-type code.
    pub type_code: u16,
    /// The entry's declared value count (untrusted — the element size is unknown).
    pub count: u64,
}

/// A recognised but out-of-spec or unparsable element a deconstruct flags.
///
/// Non-exhaustive (as are its variants): the diagnostic taxonomy grows with the deconstruct.
/// The `detail` strings are human-readable diagnostics; their exact wording is not part of the
/// API contract — match on the variant and its typed fields instead.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Anomaly {
    /// A tag whose value uses a code this crate does not recognise (e.g. an unknown `Compression`).
    #[non_exhaustive]
    UnknownCode {
        /// The page the tag was found in.
        page: usize,
        /// The tag carrying the code.
        tag: u16,
        /// The unrecognised code.
        code: u32,
        /// A human-readable description (wording not contractual).
        detail: &'static str,
    },
    /// A known tag whose value could not be interpreted (wrong type, count, or range).
    #[non_exhaustive]
    UnparsableTag {
        /// The page the tag was found in.
        page: usize,
        /// The tag.
        tag: u16,
        /// A human-readable description (wording not contractual).
        detail: &'static str,
    },
    /// An out-of-spec or unexpected structural condition.
    #[non_exhaustive]
    Structure {
        /// The page the condition relates to.
        page: usize,
        /// A human-readable description (wording not contractual).
        detail: &'static str,
        /// How serious the condition is.
        severity: Severity,
    },
}

/// The result of a strict deconstruct: byte-level classification plus DNG-specific findings.
///
/// Non-exhaustive: report categories may be added without a breaking change.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DeconstructReport {
    /// The byte-level verdict: every byte of the file typed ([`SegmentReport::segments`]), or
    /// precisely what was not — unclassified ranges, conflicts, out-of-bounds claims, and the
    /// dual-ledger parser cross-check.
    pub segments: SegmentReport,
    /// IFD entries whose field-type code was unrecognised (preserved verbatim in the parse).
    pub unknown_fields: Vec<UnknownFieldType>,
    /// Tags not part of the recognised DNG tag set.
    pub unknown_tags: Vec<UnknownTag>,
    /// Recognised-but-out-of-spec codes, unparsable tags, and structural problems.
    pub anomalies: Vec<Anomaly>,
}

impl DeconstructReport {
    /// Whether every byte of the file maps to exactly one typed segment (delegates to
    /// [`SegmentReport::is_fully_classified`] — zero tolerance, alignment padding included).
    /// Independent of whether every tag/code was recognised.
    #[must_use]
    pub fn is_fully_classified(&self) -> bool {
        self.segments.is_fully_classified()
    }

    /// Whether the file is pristine for archival: fully classified, with no unknown field
    /// types, no unknown tags, and no anomalies.
    #[must_use]
    pub fn is_fully_accounted(&self) -> bool {
        self.segments.is_fully_classified()
            && self.unknown_fields.is_empty()
            && self.unknown_tags.is_empty()
            && self.anomalies.is_empty()
    }
}

/// Walks `data` (a DNG file) and returns a [`DeconstructReport`] classifying every byte — without
/// decoding the raw image.
///
/// # Errors
///
/// Returns [`gamut_core::Error::InvalidInput`] only when the container is unreadable (bad header,
/// truncated/looping IFD chain) — the same conditions under which [`gamut_ifd::read`] fails.
/// Unknown tags, unknown field types, out-of-spec codes, malformed matrices, and unclassified
/// bytes are reported, not errored.
pub fn deconstruct(data: &[u8]) -> Result<DeconstructReport> {
    // The standard audit spec covers everything DNG locates structurally: the pointer tags
    // (SubIFDs/Exif/GPS/Interop) and the data extents (strips, tiles, free ranges, embedded
    // JPEG) — u64-native, so BigTIFF DNGs past 4 GiB account correctly.
    let audit = ifd_audit(data, &StandardAuditSpec)?;
    let mut findings = Findings::default();
    for (page, ifd) in audit.file.ifds.iter().enumerate() {
        findings.check_image_ifd(ifd, page);
    }
    findings.map_audit_findings(&audit.findings);
    Ok(DeconstructReport {
        segments: audit.report,
        unknown_fields: findings.unknown_fields,
        unknown_tags: findings.unknown_tags,
        anomalies: findings.anomalies,
    })
}

/// Accumulates the DNG-level findings over the audited tree.
#[derive(Default)]
struct Findings {
    unknown_fields: Vec<UnknownFieldType>,
    unknown_tags: Vec<UnknownTag>,
    anomalies: Vec<Anomaly>,
}

impl Findings {
    /// Checks an image IFD (IFD 0, the raw sub-IFD, or a preview/image sub-IFD): its tags,
    /// codes, colour matrices, and pixel-structure coherence, then its image sub-IFDs. Metadata
    /// sub-IFDs (Exif/GPS/Interop) belong to a separate tag namespace, so they are followed
    /// (their bytes are already classified by the audit) but not tag-checked.
    fn check_image_ifd(&mut self, ifd: &Ifd, page: usize) {
        self.check_tags(ifd, page);
        self.check_codes(ifd, page);
        self.check_matrices(ifd, page);
        self.check_pixel_structure(ifd, page);
        for group in ifd.sub_ifds() {
            if group.tag == tags::SUB_IFDS {
                for child in &group.ifds {
                    self.check_image_ifd(child, page);
                }
            }
        }
    }

    /// Flags unknown-type entries (preserved verbatim by the parse) and every field whose tag
    /// is not part of the recognised DNG tag set.
    fn check_tags(&mut self, ifd: &Ifd, page: usize) {
        for field in ifd.fields() {
            if let Value::Unknown(u) = &field.value {
                self.unknown_fields.push(UnknownFieldType {
                    page,
                    tag: field.tag,
                    type_code: u.type_code(),
                    count: u.count(),
                });
            } else if !tags::is_known_tag(field.tag) {
                self.unknown_tags.push(UnknownTag {
                    page,
                    tag: field.tag,
                    field_type: field.value.type_code(),
                    count: field.value.count(),
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

    /// Checks the pixel-data structure: the strip/tile extents themselves are claimed by the
    /// audit; this validates their *coherence* (pair present, lengths matching) and flags an
    /// image IFD with no pixel data at all.
    fn check_pixel_structure(&mut self, ifd: &Ifd, page: usize) {
        if ifd.get(tags::TILE_WIDTH).is_some() || ifd.get(tags::TILE_OFFSETS).is_some() {
            self.check_pair(
                ifd,
                page,
                tags::TILE_OFFSETS,
                tags::TILE_BYTE_COUNTS,
                "DNG: TileOffsets/TileByteCounts length mismatch",
                "DNG: tiled image missing TileOffsets/TileByteCounts",
            );
        } else if ifd.get(tags::STRIP_OFFSETS).is_some() {
            self.check_pair(
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

    /// Validates an `(offsets, byte counts)` tag pair, u64-native.
    fn check_pair(
        &mut self,
        ifd: &Ifd,
        page: usize,
        off_tag: u16,
        cnt_tag: u16,
        mismatch: &'static str,
        missing: &'static str,
    ) {
        let (Some(offsets), Some(counts)) = (ifd.get_u64_vec(off_tag), ifd.get_u64_vec(cnt_tag))
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
    }

    /// Maps the audit walk's lenient findings onto this crate's anomaly taxonomy.
    fn map_audit_findings(&mut self, findings: &[AuditFinding]) {
        for finding in findings {
            match *finding {
                AuditFinding::SkippedSubIfd { page, reason, .. } => {
                    self.anomalies.push(Anomaly::Structure {
                        page,
                        detail: match reason {
                            SkipReason::Cycle => "DNG: sub-IFD offset revisited (possible cycle)",
                            SkipReason::TooDeep => "DNG: sub-IFD nesting too deep",
                            _ => "DNG: sub-IFD could not be parsed",
                        },
                        severity: Severity::Error,
                    });
                }
                AuditFinding::ChainedSubIfd { page, .. } => {
                    self.anomalies.push(Anomaly::Structure {
                        page,
                        detail: "DNG: sub-IFD carries a next-IFD chain",
                        severity: Severity::Warning,
                    });
                }
                // The finding taxonomy may grow; unrecognised findings are not anomalies.
                _ => {}
            }
        }
    }
}

impl DngDecoder {
    /// Decodes `data` to its [`DecodedDng`] *and* returns a whole-file [`DeconstructReport`] that
    /// classifies every byte and flags anything unrecognised.
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
