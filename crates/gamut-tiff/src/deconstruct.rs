//! Strict "deconstruct" decoding: walk the whole TIFF and prove every byte was accounted for.
//!
//! Ordinary decoding reads the strips/tiles and tags it needs and ignores the rest. For archival
//! / critical use [`deconstruct`] walks the entire container — the header, every page IFD and
//! image sub-IFD, every strip/tile byte range, and the Exif/GPS metadata sub-IFDs — marking each
//! consumed byte into a [`gamut_ifd::Coverage`] and layering TIFF tag knowledge on top. The
//! resulting [`DeconstructReport`] surfaces unaccounted bytes (gaps / trailing / overlaps),
//! unknown field types, unknown/private tags, and out-of-spec codes instead of silently dropping
//! them.
//!
//! It is **collect-and-report, not fail-fast**: it runs the same structural walk the decoder
//! does and returns a report for the caller to judge; it errors only when the container itself is
//! unreadable (a malformed header or a truncated IFD), exactly as [`gamut_ifd::read`] would.

use gamut_core::{ImageBuf, Result, Rgb8};
use gamut_ifd::{
    ByteOrder, Coverage, CoverageReport, Ifd, UnknownField, Variant, read_ifd_at_with_coverage,
    read_with_coverage,
};

use crate::compression::Compression;
use crate::decoder::TiffDecoder;
use crate::ifd::{PhotometricInterpretation, Predictor};
use crate::tags;

/// An upper bound on sub-IFD nesting, guarding the recursive walk against malformed/looping trees.
const MAX_SUBIFD_DEPTH: usize = 16;

/// How serious a reported [`Anomaly`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Out of spec but often benign (e.g. an image IFD carrying no pixel data).
    Warning,
    /// A structural defect (e.g. a strip-offset/byte-count mismatch or a sub-IFD cycle).
    Error,
}

/// A tag a valid TIFF would not be expected to carry — recognised structurally but not part of the
/// TIFF 6.0 tag set (see [`tags::is_known_tag`]).
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

/// The result of a strict deconstruct: byte-range accounting plus TIFF-specific findings.
#[derive(Debug, Clone)]
pub struct DeconstructReport {
    /// Which file bytes were accounted for, and the gaps/overlaps/trailing that were not.
    pub coverage: CoverageReport,
    /// IFD entries whose field-type code was unrecognised (and thus skipped by the reader).
    pub unknown_fields: Vec<UnknownField>,
    /// Tags not part of the recognised TIFF tag set.
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

/// Walks `data` and returns a [`DeconstructReport`] accounting every byte.
///
/// # Errors
///
/// Returns [`gamut_core::Error::InvalidInput`] only when the container is unreadable (bad header,
/// truncated/looping IFD chain) — the same conditions under which [`gamut_ifd::read`] fails.
/// Unknown tags, unknown field types, out-of-spec codes, and unaccounted bytes are reported, not
/// errored.
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

/// Accumulates the byte-range accounting and TIFF findings while walking the IFD tree.
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
    /// Accounts an image IFD (a page or an image sub-IFD): its tags, codes, pixel data, and the
    /// image / metadata sub-IFDs it points at.
    fn account_image_ifd(&mut self, ifd: &Ifd, page: usize, depth: usize) {
        self.check_tags(ifd, page);
        self.check_codes(ifd, page);
        self.account_pixels(ifd, page);
        self.follow_image_subifds(ifd, page, depth);
        self.follow_metadata(ifd, page, depth);
    }

    /// Flags every field whose tag is not part of the recognised TIFF tag set.
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

    /// Flags `Compression`, `PhotometricInterpretation`, and `Predictor` values whose code this
    /// crate does not recognise.
    fn check_codes(&mut self, ifd: &Ifd, page: usize) {
        if let Some(code) = ifd.get_u32(tags::COMPRESSION)
            && Compression::try_from(code).is_err()
        {
            self.anomalies.push(Anomaly::UnknownCode {
                page,
                tag: tags::COMPRESSION,
                code,
                detail: "TIFF: unrecognised Compression code",
            });
        }
        if let Some(code) = ifd.get_u32(tags::PHOTOMETRIC_INTERPRETATION)
            && PhotometricInterpretation::try_from(code).is_err()
        {
            self.anomalies.push(Anomaly::UnknownCode {
                page,
                tag: tags::PHOTOMETRIC_INTERPRETATION,
                code,
                detail: "TIFF: unrecognised PhotometricInterpretation code",
            });
        }
        if let Some(code) = ifd.get_u32(tags::PREDICTOR)
            && Predictor::try_from(code).is_err()
        {
            self.anomalies.push(Anomaly::UnknownCode {
                page,
                tag: tags::PREDICTOR,
                code,
                detail: "TIFF: unrecognised Predictor code",
            });
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
                "TIFF: TileOffsets/TileByteCounts length mismatch",
                "TIFF: tiled image missing TileOffsets/TileByteCounts",
            );
        } else if ifd.get(tags::STRIP_OFFSETS).is_some() {
            self.account_ranges(
                ifd,
                page,
                tags::STRIP_OFFSETS,
                tags::STRIP_BYTE_COUNTS,
                "TIFF: StripOffsets/StripByteCounts length mismatch",
                "TIFF: image missing StripOffsets/StripByteCounts",
            );
        } else if ifd.get_u32(tags::IMAGE_WIDTH).is_some() {
            self.anomalies.push(Anomaly::Structure {
                page,
                detail: "TIFF: image IFD has no strip or tile data",
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

    /// Follows the `SubIFDs` (330) pointers, accounting each child as another image IFD.
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

    /// Follows the Exif (34665) and GPS (34853) sub-IFD pointers, accounting their bytes (their
    /// tags belong to a separate namespace, so they are not tag-checked here).
    fn follow_metadata(&mut self, ifd: &Ifd, page: usize, depth: usize) {
        for tag in [tags::EXIF_IFD, tags::GPS_INFO] {
            if let Some(off) = ifd.get_u32(tag) {
                self.account_meta_tree(u64::from(off), page, depth + 1);
            }
        }
    }

    /// Accounts a metadata sub-IFD and any nested Interoperability sub-IFD.
    fn account_meta_tree(&mut self, offset: u64, page: usize, depth: usize) {
        if !self.guard(offset, page, depth) {
            return;
        }
        let Some(sub) = self.read_sub(offset, page) else {
            return;
        };
        if let Some(off) = sub.get_u32(tags::INTEROPERABILITY_IFD) {
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
                    detail: "TIFF: sub-IFD could not be parsed",
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
                detail: "TIFF: sub-IFD nesting too deep",
                severity: Severity::Error,
            });
            return false;
        }
        if self.visited.contains(&offset) {
            self.anomalies.push(Anomaly::Structure {
                page,
                detail: "TIFF: sub-IFD offset revisited (possible cycle)",
                severity: Severity::Error,
            });
            return false;
        }
        self.visited.push(offset);
        true
    }
}

impl TiffDecoder {
    /// Strictly deconstructs `data`, returning a [`DeconstructReport`] that accounts every byte and
    /// flags anything unrecognised — without decoding pixels.
    ///
    /// # Errors
    ///
    /// Returns [`gamut_core::Error`] only when the container itself is unreadable (see
    /// [`deconstruct`]).
    pub fn deconstruct(&self, data: &[u8]) -> Result<DeconstructReport> {
        deconstruct(data)
    }

    /// Decodes page `page` to [`Rgb8`] *and* returns a whole-file [`DeconstructReport`].
    ///
    /// The report covers the entire file (every page), independent of which page is decoded.
    ///
    /// # Errors
    ///
    /// Returns [`gamut_core::Error`] if the container is unreadable or the requested page cannot be
    /// decoded (e.g. an unsupported pixel mode) — a strict superset of [`TiffDecoder::decode_page`].
    pub fn deconstruct_page(
        &self,
        data: &[u8],
        page: usize,
    ) -> Result<(ImageBuf<Rgb8>, DeconstructReport)> {
        let report = deconstruct(data)?;
        let image = self.decode_page(data, page)?;
        Ok((image, report))
    }
}

#[cfg(test)]
mod tests {
    use gamut_core::{Dimensions, EncodeImage, Gray8, ImageRef};
    use gamut_ifd::Value;

    use super::*;
    use crate::encoder::TiffEncoder;
    use crate::writer::{write_image, write_multipage};

    /// Total bytes left in interior gaps — for a tightly-written file this is only word-alignment
    /// padding (at most one byte before the strip data).
    fn total_gap_bytes(r: &DeconstructReport) -> u64 {
        r.coverage.gaps.iter().map(|g| g.len).sum()
    }

    /// Asserts the structural walk explained the whole file bar word-alignment padding.
    fn assert_clean_coverage(r: &DeconstructReport) {
        assert!(r.coverage.overlaps.is_empty(), "overlaps: {r:?}");
        assert!(r.coverage.out_of_bounds.is_empty(), "out of bounds: {r:?}");
        assert!(r.coverage.trailing.is_none(), "trailing: {r:?}");
        assert!(r.unknown_fields.is_empty(), "unknown fields: {r:?}");
        assert!(total_gap_bytes(r) <= 2, "alignment gap too large: {r:?}");
    }

    /// A minimal 2×2 8-bit BlackIsZero grayscale image IFD (the strip tags are filled in by the
    /// writer).
    fn image_ifd() -> Ifd {
        let mut ifd = Ifd::new();
        ifd.set(tags::IMAGE_WIDTH, Value::Short(vec![2]));
        ifd.set(tags::IMAGE_LENGTH, Value::Short(vec![2]));
        ifd.set(tags::BITS_PER_SAMPLE, Value::Short(vec![8]));
        ifd.set(tags::PHOTOMETRIC_INTERPRETATION, Value::Short(vec![1]));
        ifd.set(tags::SAMPLES_PER_PIXEL, Value::Short(vec![1]));
        ifd.set(tags::ROWS_PER_STRIP, Value::Short(vec![2]));
        ifd
    }

    #[test]
    fn encoded_gray_strip_image_is_accounted() {
        for order in [ByteOrder::LittleEndian, ByteOrder::BigEndian] {
            let dims = Dimensions {
                width: 5,
                height: 3,
            };
            let pixels: Vec<u8> = (0..15).collect();
            let mut tiff = Vec::new();
            TiffEncoder::new()
                .with_byte_order(order)
                .encode_image(ImageRef::<Gray8>::new(&pixels, dims).unwrap(), &mut tiff)
                .expect("encode");
            let report = deconstruct(&tiff).expect("deconstruct");
            assert_clean_coverage(&report);
            assert!(report.unknown_tags.is_empty(), "{report:?}");
            assert!(report.anomalies.is_empty(), "{report:?}");
        }
    }

    #[test]
    fn flags_unknown_private_tag() {
        let mut ifd = image_ifd();
        ifd.set(0x9999, Value::Long(vec![42])); // private/unknown tag, inline LONG
        let bytes = write_image(
            ByteOrder::LittleEndian,
            Variant::Classic,
            &ifd,
            &[vec![0u8; 4]],
        )
        .expect("write");
        let report = deconstruct(&bytes).expect("deconstruct");
        assert_clean_coverage(&report); // the unknown tag's bytes are still covered
        assert!(
            report
                .unknown_tags
                .iter()
                .any(|u| u.tag == 0x9999 && u.page == 0),
            "{report:?}"
        );
        // Unknown tags fail the strict verdict but not the byte-coverage one.
        assert!(!report.is_fully_accounted());
    }

    #[test]
    fn flags_unknown_compression_code() {
        let mut ifd = image_ifd();
        ifd.set(tags::COMPRESSION, Value::Short(vec![999])); // not a recognised Compression code
        let bytes = write_image(
            ByteOrder::LittleEndian,
            Variant::Classic,
            &ifd,
            &[vec![0u8; 4]],
        )
        .expect("write");
        let report = deconstruct(&bytes).expect("deconstruct");
        assert!(
            report.anomalies.iter().any(|a| matches!(
                a,
                Anomaly::UnknownCode { tag, code, .. }
                    if *tag == tags::COMPRESSION && *code == 999
            )),
            "{report:?}"
        );
    }

    #[test]
    fn flags_strip_offset_count_mismatch() {
        // Two offsets but one byte count: a structural defect the deconstruct must surface.
        let mut ifd = image_ifd();
        ifd.set(tags::STRIP_OFFSETS, Value::Long(vec![1000, 2000]));
        ifd.set(tags::STRIP_BYTE_COUNTS, Value::Long(vec![4]));
        // Write with no real strips so the writer does not overwrite our deliberate mismatch.
        let bytes = gamut_ifd::write(&gamut_ifd::TiffFile {
            order: ByteOrder::LittleEndian,
            variant: Variant::Classic,
            ifds: vec![ifd],
        })
        .expect("write");
        let report = deconstruct(&bytes).expect("deconstruct");
        assert!(
            report.anomalies.iter().any(|a| matches!(
                a,
                Anomaly::Structure {
                    severity: Severity::Error,
                    ..
                }
            )),
            "{report:?}"
        );
    }

    #[test]
    fn multipage_is_accounted() {
        let pages = [
            (image_ifd(), vec![vec![0u8; 4]]),
            (image_ifd(), vec![vec![1u8; 4]]),
        ];
        let bytes =
            write_multipage(ByteOrder::LittleEndian, Variant::Classic, &pages).expect("write");
        let report = deconstruct(&bytes).expect("deconstruct");
        assert_clean_coverage(&report);
        assert!(report.unknown_tags.is_empty(), "{report:?}");
        assert!(report.anomalies.is_empty(), "{report:?}");
    }

    #[test]
    fn follows_and_accounts_an_exif_subifd() {
        // The writer lays out an Exif sub-IFD reached through the ExifIFD pointer; the deconstruct
        // must follow it and account its bytes, and must NOT mis-flag its EXIF-namespace tag
        // (ExposureTime, 33434) as an unknown TIFF tag.
        let mut exif = Ifd::new();
        exif.set(33434, Value::Rational(vec![(1, 100)])); // ExposureTime — EXIF namespace
        let mut ifd = image_ifd();
        ifd.set_sub_ifd(tags::EXIF_IFD, vec![exif]);
        let bytes = write_image(
            ByteOrder::LittleEndian,
            Variant::Classic,
            &ifd,
            &[vec![0u8; 4]],
        )
        .expect("write");
        let report = deconstruct(&bytes).expect("deconstruct");
        assert_clean_coverage(&report);
        assert!(report.unknown_tags.is_empty(), "{report:?}");
        assert!(report.anomalies.is_empty(), "{report:?}");
    }

    #[test]
    fn image_ifd_without_pixel_data_warns() {
        // An image IFD (has ImageWidth) with neither strips nor tiles is flagged as a warning.
        let mut ifd = Ifd::new();
        ifd.set(tags::IMAGE_WIDTH, Value::Short(vec![2]));
        ifd.set(tags::IMAGE_LENGTH, Value::Short(vec![2]));
        let bytes = gamut_ifd::write(&gamut_ifd::TiffFile {
            order: ByteOrder::LittleEndian,
            variant: Variant::Classic,
            ifds: vec![ifd],
        })
        .expect("write");
        let report = deconstruct(&bytes).expect("deconstruct");
        assert!(
            report.anomalies.iter().any(|a| matches!(
                a,
                Anomaly::Structure {
                    severity: Severity::Warning,
                    ..
                }
            )),
            "{report:?}"
        );
    }
}
