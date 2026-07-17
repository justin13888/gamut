//! The shared structural byte-completeness auditor (issue #263).
//!
//! [`audit`] walks a whole TIFF-family stream — header, the top-level IFD chain, every sub-IFD
//! reached through the spec's pointer tags, and every data extent the spec's tags locate — over
//! a [`Tracked`] source, collecting typed claims into a [`SegmentMap`] and finishing with the
//! dual-ledger cross-check. The result classifies **every byte** of the file, or reports
//! precisely what could not be classified.
//!
//! The walk is **lenient where the file is at fault** (an unparseable or cyclic sub-IFD becomes
//! an [`AuditFinding`], its bytes simply stay unclassified) and **strict where the parser is at
//! fault** (the dual-ledger invariants). It errors only where [`read`](crate::read) would: an
//! unreadable header or top-level chain — or on a transport failure.
//!
//! Codecs supply tag knowledge through [`AuditSpec`]; the [`StandardAuditSpec`] covers the
//! structural tags every TIFF-family file shares (strips, tiles, free ranges, embedded JPEG).

use gamut_core::{Error, Result};

use crate::reader::pointer_offsets;
use crate::segment::{Claim, DataLabel, SegmentMap, SegmentReport, SpanKind};
use crate::source::ReadAt;
use crate::stream::IfdReader;
use crate::track::Tracked;
use crate::{Ifd, TiffFile, tags};

/// An upper bound on sub-IFD nesting, guarding the recursive walk against hostile trees. The
/// deepest legitimate trees (a DNG raw sub-IFD, EXIF's Exif → Interop chain) are three levels.
const MAX_SUBIFD_DEPTH: usize = 16;

/// The codec-supplied tag knowledge an [`audit`] walk needs. Everything has a default, so a
/// plain-TIFF audit is [`StandardAuditSpec`]; a codec overrides to add its own pointer tags or
/// data carriers.
pub trait AuditSpec {
    /// The pointer tags followed as sub-IFD graphs (default:
    /// [`tags::STANDARD_POINTER_TAGS`]).
    fn pointer_tags(&self) -> &[u16] {
        tags::STANDARD_POINTER_TAGS
    }

    /// Reports every data extent `ifd`'s tag values locate, as `sink(offset, len, label)`
    /// calls. The default reports the standard structural carriers via
    /// [`standard_data_extents`]; an override that still wants those should call it too.
    fn data_extents(&self, ifd: &Ifd, sink: &mut dyn FnMut(u64, u64, DataLabel)) {
        standard_data_extents(ifd, sink);
    }
}

/// The default [`AuditSpec`]: standard pointer tags, standard data extents.
#[derive(Debug, Clone, Copy, Default)]
pub struct StandardAuditSpec;

impl AuditSpec for StandardAuditSpec {}

/// Reports the data extents the **structural** tag pairs locate — strips, tiles, free ranges,
/// and the embedded-JPEG range — as `sink(offset, len, label)` calls, u64-native (BigTIFF
/// extents past 4 GiB coerce losslessly).
///
/// Offset/count arrays are paired index-wise; a length mismatch leaves the unpaired tail
/// unclaimed (so its bytes surface as unclassified — the codec's audit layer reports the
/// mismatch itself).
pub fn standard_data_extents(ifd: &Ifd, sink: &mut dyn FnMut(u64, u64, DataLabel)) {
    let pairs = [
        (
            tags::STRIP_OFFSETS,
            tags::STRIP_BYTE_COUNTS,
            DataLabel::Strip,
        ),
        (tags::TILE_OFFSETS, tags::TILE_BYTE_COUNTS, DataLabel::Tile),
        (
            tags::JPEG_INTERCHANGE_FORMAT,
            tags::JPEG_INTERCHANGE_FORMAT_LENGTH,
            DataLabel::JpegInterchange,
        ),
        (tags::FREE_OFFSETS, tags::FREE_BYTE_COUNTS, DataLabel::Free),
    ];
    for (off_tag, len_tag, label) in pairs {
        let (Some(offsets), Some(lens)) = (ifd.get_u64_vec(off_tag), ifd.get_u64_vec(len_tag))
        else {
            continue;
        };
        for (&offset, &len) in offsets.iter().zip(&lens) {
            sink(offset, len, label);
        }
    }
}

/// A lenient observation the audit walk made about the *file* (as opposed to the byte-level
/// verdicts in the [`SegmentReport`]).
///
/// `#[non_exhaustive]`: further findings can be added without a breaking change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuditFinding {
    /// A sub-IFD pointer target that could not be followed: unparseable, revisited (a cycle),
    /// or nested beyond the depth guard. Its bytes stay unclassified.
    SkippedSubIfd {
        /// The pointer tag that named the target.
        tag: u16,
        /// The target offset.
        offset: u64,
    },
    /// A followed sub-IFD carried a non-zero next-IFD link — out of spec for the standard
    /// pointer tags. The chained directory was followed, accounted, and appended to the same
    /// pointer group.
    ChainedSubIfd {
        /// The pointer tag whose target carried the chain.
        tag: u16,
        /// The directory whose next-IFD link was non-zero.
        offset: u64,
    },
}

/// A completed structural audit: the parsed tree, the byte-level report, and the walk's
/// lenient findings.
#[derive(Debug, Clone)]
pub struct Audit {
    /// The parsed stream, with the spec's pointer tags resolved into sub-IFD groups (like
    /// [`read_tree`](crate::read_tree); a skipped target leaves its group incomplete — see
    /// [`AuditFinding::SkippedSubIfd`]).
    pub file: TiffFile,
    /// The byte-level verdict: every byte classified, or precisely what was not.
    pub report: SegmentReport,
    /// Lenient file-level observations made during the walk.
    pub findings: Vec<AuditFinding>,
}

/// Audits `source` for byte completeness: parses the whole tree under a [`Tracked`] read
/// ledger, claims every structure and declared data extent, classifies padding from the actual
/// bytes, and cross-checks the dual-ledger invariants.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] only when the container is unreadable (bad header,
/// unreadable top-level chain) — the same conditions as [`read`](crate::read) — or
/// [`Error::Io`] on a transport failure. File defects inside sub-IFDs are findings, not errors.
pub fn audit<S: ReadAt>(source: S, spec: &dyn AuditSpec) -> Result<Audit> {
    let mut tracked = Tracked::new(source);
    let file_len = tracked.len()?;
    let mut map = SegmentMap::new(file_len);
    let mut findings = Vec::new();

    let mut file = {
        let mut reader = IfdReader::open(&mut tracked)?;
        let mut file = reader.read_file_audited(&mut map)?;
        let mut visited: Vec<u64> = Vec::new();
        for ifd in &mut file.ifds {
            declare_extents(spec, ifd, &mut map);
            follow_subifds(
                &mut reader,
                &mut map,
                spec,
                ifd,
                &mut visited,
                1,
                &mut findings,
            )?;
        }
        file
    };

    let mut report = map.finish(Some(tracked.ledger()));
    report.classify_padding(&mut tracked)?;
    // Canonicalise the resolved tree the way `read_tree` does (sorted groups via set_sub_ifd —
    // already guaranteed) so `audit(...).file == read_tree(...)` for clean files.
    file.ifds.shrink_to_fit();
    Ok(Audit {
        file,
        report,
        findings,
    })
}

/// Claims every data extent the spec locates in `ifd`, as [`Claim::Declared`].
fn declare_extents(spec: &dyn AuditSpec, ifd: &Ifd, map: &mut SegmentMap) {
    spec.data_extents(ifd, &mut |offset, len, label| {
        map.claim(offset, len, SpanKind::Data(label), Claim::Declared);
    });
}

/// Follows `ifd`'s pointer tags leniently: each parseable target becomes a sub-IFD child
/// (its extents declared, its own pointers followed recursively); an unparseable, cyclic, or
/// too-deep target becomes a [`AuditFinding::SkippedSubIfd`]; a non-zero next-IFD link on a
/// target is followed as [`AuditFinding::ChainedSubIfd`].
fn follow_subifds<S: ReadAt>(
    reader: &mut IfdReader<S>,
    map: &mut SegmentMap,
    spec: &dyn AuditSpec,
    ifd: &mut Ifd,
    visited: &mut Vec<u64>,
    depth: usize,
    findings: &mut Vec<AuditFinding>,
) -> Result<()> {
    for &tag in spec.pointer_tags() {
        let Some(offsets) = ifd.get(tag).and_then(pointer_offsets) else {
            continue;
        };
        let mut children: Vec<Ifd> = Vec::with_capacity(offsets.len());
        for head in offsets {
            let mut offset = head;
            while offset != 0 {
                if depth + 1 > MAX_SUBIFD_DEPTH || visited.contains(&offset) {
                    findings.push(AuditFinding::SkippedSubIfd { tag, offset });
                    break;
                }
                visited.push(offset);
                let (mut child, next) = match reader.read_ifd_at_audited(offset, map) {
                    Ok(parsed) => parsed,
                    // Transport failures abort the audit; file defects are findings.
                    Err(e @ Error::Io(_)) => return Err(e),
                    Err(_) => {
                        findings.push(AuditFinding::SkippedSubIfd { tag, offset });
                        break;
                    }
                };
                declare_extents(spec, &child, map);
                follow_subifds(reader, map, spec, &mut child, visited, depth + 1, findings)?;
                children.push(child);
                if next != 0 {
                    findings.push(AuditFinding::ChainedSubIfd { tag, offset });
                }
                offset = next;
            }
        }
        if !children.is_empty() {
            ifd.remove(tag);
            ifd.set_sub_ifd(tag, children);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ByteOrder, Value, Variant, align_word, read_tree, write};

    fn classic_le(ifds: Vec<Ifd>) -> Vec<u8> {
        write(&TiffFile {
            order: ByteOrder::LittleEndian,
            variant: Variant::Classic,
            ifds,
        })
        .expect("write")
    }

    /// A written tree with appended strip data audits to full classification: structure via
    /// Parsed claims, strips via Declared extents, alignment via padding classification.
    #[test]
    fn audit_classifies_a_tree_with_strip_data() {
        let strip = [9u8, 8, 7, 6, 5];
        let build = |strip_at: u64| {
            let mut exif = Ifd::new();
            exif.set(33434, Value::Rational(vec![(1, 200)]));
            let mut root = Ifd::new();
            root.set(256, Value::Short(vec![640]));
            root.set(270, Value::Ascii("odd".to_owned())); // odd length: forces padding
            root.set(273, Value::Long(vec![strip_at as u32]));
            root.set(279, Value::Long(vec![strip.len() as u32]));
            root.set_sub_ifd(tags::EXIF_IFD, vec![exif]);
            root
        };
        // Structural determinism: measure with a placeholder, then write the real offset.
        let probe = classic_le(vec![build(0)]);
        let strip_at = align_word(probe.len() as u64);
        let mut bytes = classic_le(vec![build(strip_at)]);
        bytes.resize(strip_at as usize, 0);
        bytes.extend_from_slice(&strip);

        let out = audit(&bytes[..], &StandardAuditSpec).expect("audit");
        assert!(out.findings.is_empty(), "findings: {:?}", out.findings);
        assert!(out.report.is_fully_classified(), "report: {:?}", out.report);
        // The tree matches read_tree, and the strip extent is a Declared Data segment.
        assert_eq!(
            out.file,
            read_tree(&bytes, &[tags::EXIF_IFD]).expect("tree")
        );
        assert!(out.report.segments.iter().any(|s| {
            s.kind == SpanKind::Data(DataLabel::Strip)
                && s.range
                    == crate::Range {
                        start: strip_at,
                        len: strip.len() as u64,
                    }
        }));
    }

    /// An unparseable sub-IFD target is a lenient finding — the audit completes and the
    /// target's bytes stay unclassified.
    #[test]
    fn unparseable_subifd_is_a_finding_not_an_error() {
        let mut root = Ifd::new();
        root.set(256, Value::Short(vec![640]));
        // A pointer aimed just past the end of the file.
        root.set(tags::SUB_IFDS, Value::Long(vec![0xFFFF]));
        let bytes = classic_le(vec![root]);
        let out = audit(&bytes[..], &StandardAuditSpec).expect("audit");
        assert_eq!(
            out.findings,
            vec![AuditFinding::SkippedSubIfd {
                tag: tags::SUB_IFDS,
                offset: 0xFFFF,
            }]
        );
        // The stale pointer stays in place as a field (nothing was resolved).
        assert!(out.file.ifds[0].get(tags::SUB_IFDS).is_some());
        assert!(
            out.report.is_fully_classified(),
            "the file bytes themselves are all classified"
        );
    }

    /// A self-pointing sub-IFD terminates as a cycle finding, not a hang.
    #[test]
    fn cyclic_subifd_is_a_finding() {
        // Root @8 points at the child @26, whose own pointer aims back at 26.
        let data: &[u8] = &[
            b'I', b'I', 0x2a, 0x00, 0x08, 0x00, 0x00, 0x00, //
            0x01, 0x00, 0x4a, 0x01, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00, 0x1a, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, //
            0x01, 0x00, 0x4a, 0x01, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00, 0x1a, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
        ];
        let out = audit(data, &StandardAuditSpec).expect("audit");
        assert_eq!(
            out.findings,
            vec![AuditFinding::SkippedSubIfd {
                tag: tags::SUB_IFDS,
                offset: 26,
            }]
        );
        // The child itself was parsed and accounted before the cycle was refused.
        assert_eq!(out.file.ifds[0].sub_ifds().len(), 1);
        assert!(out.report.is_fully_classified(), "report: {:?}", out.report);
    }

    /// A sub-IFD carrying an out-of-spec next-IFD chain link: the chained directory is
    /// followed, accounted, and appended to the same group, with a finding.
    #[test]
    fn chained_subifd_is_followed_and_flagged() {
        let data: &[u8] = &[
            b'I', b'I', 0x2a, 0x00, 0x08, 0x00, 0x00, 0x00, // header, IFD0 @ 8
            0x01, 0x00, // IFD0: 1 entry
            0x4a, 0x01, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00, 0x1a, 0x00, 0x00,
            0x00, // 330 -> 26
            0x00, 0x00, 0x00, 0x00, // next = 0
            0x01, 0x00, // child A @ 26: 1 entry
            0x00, 0x01, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, // 256 = 1
            0x2c, 0x00, 0x00, 0x00, // next = 44 (out of spec for SubIFDs)
            0x01, 0x00, // child B @ 44: 1 entry
            0x00, 0x01, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, // 256 = 2
            0x00, 0x00, 0x00, 0x00, // next = 0
        ];
        let out = audit(data, &StandardAuditSpec).expect("audit");
        assert_eq!(
            out.findings,
            vec![AuditFinding::ChainedSubIfd {
                tag: tags::SUB_IFDS,
                offset: 26,
            }]
        );
        let group = &out.file.ifds[0].sub_ifds()[0];
        assert_eq!(group.ifds.len(), 2, "the chained directory joins the group");
        assert_eq!(group.ifds[0].get_u32(256), Some(1));
        assert_eq!(group.ifds[1].get_u32(256), Some(2));
        assert!(out.report.is_fully_classified(), "report: {:?}", out.report);
    }

    /// Nesting past the depth guard terminates with skip findings, not a stack overflow.
    #[test]
    fn depth_bomb_is_skipped_leniently() {
        let mut ifd = Ifd::new();
        ifd.set(256, Value::Short(vec![1]));
        for _ in 0..20 {
            let mut parent = Ifd::new();
            parent.set_sub_ifd(tags::SUB_IFDS, vec![ifd]);
            ifd = parent;
        }
        let bytes = classic_le(vec![ifd]);
        let out = audit(&bytes[..], &StandardAuditSpec).expect("audit");
        assert!(
            out.findings
                .iter()
                .any(|f| matches!(f, AuditFinding::SkippedSubIfd { .. })),
            "the too-deep tail is skipped"
        );
        // The skipped tail's directory bytes are unclassified — the honest verdict.
        assert!(!out.report.is_fully_classified());
    }

    /// An unreadable header is still a hard error, exactly like `read`.
    #[test]
    fn unreadable_container_errors() {
        assert!(audit(&b"not a tiff"[..], &StandardAuditSpec).is_err());
        assert!(audit(&b""[..], &StandardAuditSpec).is_err());
    }
}
