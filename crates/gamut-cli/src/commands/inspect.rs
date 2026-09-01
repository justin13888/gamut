//! `gamut inspect` — strict "deconstruct" of a TIFF, DNG or PNG (issues #197/#263/#224).
//!
//! Walks the entire container, classifies every byte into typed segments, and flags anything
//! unrecognised (unknown tags, unknown field types, out-of-spec codes, unclassified bytes).
//! Prints a report to stdout and exits non-zero when the file is not fully accounted for —
//! usable as an archival CI gate.
//!
//! For PNG the same walk answers a second question: **where did the bytes go?** The report carries
//! the per-chunk-type breakdown, the compressed IDAT total against the filtered stream it inflates
//! to, and the scanline filter distribution — which is what makes an encoder comparison possible
//! from the command line, on files this crate did not write.

use std::path::PathBuf;

use clap::{Args, ValueEnum};
use gamut::ifd::SegmentReport;

use crate::error::CliError;

/// The most list entries to print per category before truncating.
const MAX_LIST: usize = 20;

/// The `DNGVersion` tag (50706) — present in every DNG, absent in plain TIFF; the format sniff.
const DNG_VERSION_TAG: u16 = 50706;

/// Arguments for `gamut inspect`.
#[derive(Args)]
pub(crate) struct InspectArgs {
    /// Input TIFF, DNG or PNG file.
    input: PathBuf,
    /// Force the container format instead of auto-detecting it.
    #[arg(long, value_enum)]
    format: Option<Format>,
}

/// The container format to deconstruct as.
#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum Format {
    /// TIFF 6.0 / BigTIFF (gamut-tiff).
    Tiff,
    /// DNG (Adobe Digital Negative; gamut-dng).
    Dng,
    /// PNG (gamut-png).
    Png,
}

/// A format-agnostic view of a deconstruct report, for printing.
struct Summary {
    segments: SegmentReport,
    unknown_fields: Vec<String>,
    unknown_tags: Vec<String>,
    anomalies: Vec<String>,
    fully_classified: bool,
    fully_accounted: bool,
}

/// Runs the `inspect` command: deconstruct the file, print the report, and exit non-zero if it is
/// not fully accounted for.
pub(crate) fn run(args: &InspectArgs) -> Result<(), CliError> {
    let data = std::fs::read(&args.input).map_err(|source| CliError::Io {
        path: args.input.clone(),
        source,
    })?;
    let format = args.format.unwrap_or_else(|| sniff(&data));

    // PNG's report is a different shape -- it has no IFD tree and no tag vocabulary, but it does
    // carry compression figures the others have no equivalent for -- so it prints on its own path
    // rather than being flattened into `Summary`.
    if matches!(format, Format::Png) {
        return inspect_png(&args.input, &data);
    }

    let summary = match format {
        Format::Dng => summarize_dng(gamut::dng::deconstruct(&data)?),
        Format::Tiff => summarize_tiff(gamut::tiff::deconstruct(&data)?),
        Format::Png => unreachable!("handled above"),
    };

    print_summary(&args.input, format, &summary);

    if summary.fully_accounted {
        Ok(())
    } else {
        Err(CliError::NotFullyAccounted(format!(
            "{}: not fully accounted — {} unclassified byte(s), {} unknown tag(s), {} unknown field type(s), {} anomaly/ies",
            args.input.display(),
            summary.segments.unclassified_bytes(),
            summary.unknown_tags.len(),
            summary.unknown_fields.len(),
            summary.anomalies.len(),
        )))
    }
}

/// The 8-byte PNG file signature (§5.2).
const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

/// Detects PNG by signature, then DNG vs TIFF: a DNG is a TIFF whose IFD 0 carries the mandatory
/// `DNGVersion` tag.
fn sniff(data: &[u8]) -> Format {
    if data.starts_with(&PNG_SIGNATURE) {
        return Format::Png;
    }
    if let Ok(file) = gamut::tiff::read(data)
        && file
            .ifds
            .first()
            .is_some_and(|ifd0| ifd0.get(DNG_VERSION_TAG).is_some())
    {
        return Format::Dng;
    }
    Format::Tiff
}

/// Collapses a TIFF report into the printable [`Summary`].
fn summarize_tiff(report: gamut::tiff::DeconstructReport) -> Summary {
    use gamut::tiff::Anomaly;
    let fully_classified = report.is_fully_classified();
    let fully_accounted = report.is_fully_accounted();
    let unknown_fields = report
        .unknown_fields
        .iter()
        .map(|u| {
            format!(
                "page {} tag {:#06x} type {} (count {})",
                u.page, u.tag, u.type_code, u.count
            )
        })
        .collect();
    let unknown_tags = report
        .unknown_tags
        .iter()
        .map(|u| {
            format!(
                "page {} tag {:#06x} (type {}, count {})",
                u.page, u.tag, u.field_type, u.count
            )
        })
        .collect();
    let anomalies = report
        .anomalies
        .iter()
        .map(|a| match a {
            Anomaly::UnknownCode {
                page,
                tag,
                code,
                detail,
                ..
            } => {
                format!("[error] page {page}: {detail} (tag {tag:#06x}, code {code})")
            }
            Anomaly::UnparsableTag {
                page, tag, detail, ..
            } => {
                format!("[error] page {page}: {detail} (tag {tag:#06x})")
            }
            Anomaly::Structure {
                page,
                detail,
                severity,
                ..
            } => {
                format!("[{}] page {page}: {detail}", severity_label_tiff(*severity))
            }
            // `Anomaly` is non-exhaustive; render future categories generically.
            other => format!("[error] {other:?}"),
        })
        .collect();
    Summary {
        segments: report.segments,
        unknown_fields,
        unknown_tags,
        anomalies,
        fully_classified,
        fully_accounted,
    }
}

/// Collapses a DNG report into the printable [`Summary`].
fn summarize_dng(report: gamut::dng::DeconstructReport) -> Summary {
    use gamut::dng::Anomaly;
    let fully_classified = report.is_fully_classified();
    let fully_accounted = report.is_fully_accounted();
    let unknown_fields = report
        .unknown_fields
        .iter()
        .map(|u| {
            format!(
                "page {} tag {:#06x} type {} (count {})",
                u.page, u.tag, u.type_code, u.count
            )
        })
        .collect();
    let unknown_tags = report
        .unknown_tags
        .iter()
        .map(|u| {
            format!(
                "page {} tag {:#06x} (type {}, count {})",
                u.page, u.tag, u.field_type, u.count
            )
        })
        .collect();
    let anomalies = report
        .anomalies
        .iter()
        .map(|a| match a {
            Anomaly::UnknownCode {
                page,
                tag,
                code,
                detail,
                ..
            } => {
                format!("[error] page {page}: {detail} (tag {tag:#06x}, code {code})")
            }
            Anomaly::UnparsableTag {
                page, tag, detail, ..
            } => {
                format!("[error] page {page}: {detail} (tag {tag:#06x})")
            }
            Anomaly::Structure {
                page,
                detail,
                severity,
                ..
            } => {
                format!("[{}] page {page}: {detail}", severity_label_dng(*severity))
            }
            // `Anomaly` is non-exhaustive; render future categories generically.
            other => format!("[error] {other:?}"),
        })
        .collect();
    Summary {
        segments: report.segments,
        unknown_fields,
        unknown_tags,
        anomalies,
        fully_classified,
        fully_accounted,
    }
}

/// Renders a TIFF anomaly severity.
fn severity_label_tiff(severity: gamut::tiff::Severity) -> &'static str {
    match severity {
        gamut::tiff::Severity::Warning => "warning",
        gamut::tiff::Severity::Error => "error",
        // `Severity` is non-exhaustive; treat future levels as errors (the conservative label).
        _ => "error",
    }
}

/// Renders a DNG anomaly severity.
fn severity_label_dng(severity: gamut::dng::Severity) -> &'static str {
    match severity {
        gamut::dng::Severity::Warning => "warning",
        gamut::dng::Severity::Error => "error",
        // `Severity` is non-exhaustive; treat future levels as errors (the conservative label).
        _ => "error",
    }
}

/// Prints the human-readable report to stdout.
fn print_summary(path: &std::path::Path, format: Format, s: &Summary) {
    let seg = &s.segments;
    let classified = seg.file_len - seg.unclassified_bytes();
    let pct = if seg.file_len == 0 {
        100.0
    } else {
        classified as f64 / seg.file_len as f64 * 100.0
    };
    println!("inspecting {} as {}", path.display(), format_name(format));
    println!(
        "  classified:    {classified} / {} bytes ({pct:.1}%) across {} segment(s)",
        seg.file_len,
        seg.segments.len()
    );
    println!("  unclassified:  {} bytes", seg.unclassified_bytes());

    print_ranges(
        "unclassified ranges",
        &seg.unclassified
            .iter()
            .map(|g| (g.start, g.len))
            .collect::<Vec<_>>(),
    );
    if !seg.conflicts.is_empty() {
        println!("  conflicts:     {}", seg.conflicts.len());
        for c in seg.conflicts.iter().take(MAX_LIST) {
            println!(
                "    - [{}, {}) overlaps [{}, {})",
                c.b.range.start,
                c.b.range.end(),
                c.a.range.start,
                c.a.range.end()
            );
        }
    }
    if !seg.shared.is_empty() {
        println!("  shared values: {} (legal TIFF sharing)", seg.shared.len());
    }
    print_ranges(
        "out-of-bounds claims",
        &seg.out_of_bounds
            .iter()
            .map(|o| (o.range.start, o.range.len))
            .collect::<Vec<_>>(),
    );
    // The dual-ledger parser cross-check: non-empty lists here are gamut bugs, not file defects.
    print_ranges(
        "unclaimed reads (parser defect)",
        &seg.unclaimed_reads
            .iter()
            .map(|r| (r.start, r.len))
            .collect::<Vec<_>>(),
    );
    print_ranges(
        "unread claims (parser defect)",
        &seg.unread_claims
            .iter()
            .map(|c| (c.range.start, c.range.len))
            .collect::<Vec<_>>(),
    );

    print_lines("unknown field types", &s.unknown_fields);
    print_lines("unknown tags", &s.unknown_tags);
    print_lines("anomalies", &s.anomalies);

    println!("  fully classified: {}", yes_no(s.fully_classified));
    println!("  fully accounted:  {}", yes_no(s.fully_accounted));
}

/// Prints a `(start, len)` range list under `label`, truncating past [`MAX_LIST`].
fn print_ranges(label: &str, ranges: &[(u64, u64)]) {
    if ranges.is_empty() {
        return;
    }
    println!("  {label}: {}", ranges.len());
    for (start, len) in ranges.iter().take(MAX_LIST) {
        println!("    - {len} bytes at offset {start}");
    }
    if ranges.len() > MAX_LIST {
        println!("    … and {} more", ranges.len() - MAX_LIST);
    }
}

/// Prints a pre-formatted line list under `label`, truncating past [`MAX_LIST`].
fn print_lines(label: &str, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    println!("  {label}: {}", lines.len());
    for line in lines.iter().take(MAX_LIST) {
        println!("    - {line}");
    }
    if lines.len() > MAX_LIST {
        println!("    … and {} more", lines.len() - MAX_LIST);
    }
}

/// The display name of a format.
/// Deconstructs a PNG and prints where its bytes went, exiting non-zero when the file is not a
/// complete, undamaged datastream.
fn inspect_png(path: &std::path::Path, data: &[u8]) -> Result<(), CliError> {
    use gamut::png::{FilterScan, FilterType, SegmentKind};

    let report = gamut::png::deconstruct(data)?;
    let header = report.header;

    println!("{}: PNG", path.display());
    println!(
        "  image:         {}x{} {:?} depth {}{}",
        header.width,
        header.height,
        header.color_type,
        header.bit_depth,
        if header.interlaced {
            ", Adam7 interlaced"
        } else {
            ""
        }
    );
    println!(
        "  size:          {} bytes ({:.3} bits/pixel)",
        report.file_len,
        report.bits_per_pixel()
    );
    println!(
        "  IDAT:          {} bytes compressed from {} filtered ({:.1}%)",
        report.idat_compressed,
        report.filtered_len,
        report.idat_ratio() * 100.0
    );
    println!(
        "  overhead:      {} bytes, of which {} is chunk framing",
        report.overhead_bytes(),
        report.framing_bytes()
    );

    println!("  chunks:");
    for stats in &report.chunks {
        println!(
            "    {} x{:<3} {:>9} payload + {:>4} framing{}",
            String::from_utf8_lossy(&stats.chunk_type),
            stats.count,
            stats.payload_bytes,
            stats.framing_bytes(),
            if stats.is_ancillary() {
                " (ancillary)"
            } else {
                ""
            }
        );
    }

    match report.filters {
        FilterScan::Counted(h) => {
            let n = |f| h.count(f);
            println!(
                "  filters:       None {} / Sub {} / Up {} / Average {} / Paeth {}  ({} scanlines)",
                n(FilterType::None),
                n(FilterType::Sub),
                n(FilterType::Up),
                n(FilterType::Average),
                n(FilterType::Paeth),
                h.total()
            );
        }
        FilterScan::Skipped(reason) => {
            println!(
                "  filters:       not counted — {}",
                filter_skip_label(reason)
            );
        }
    }

    if report.passes.len() > 1 {
        println!("  Adam7 passes:");
        for pass in &report.passes {
            println!(
                "    {}: {}x{}, {} row bytes, {} filtered",
                pass.index, pass.width, pass.height, pass.row_bytes, pass.filtered_len
            );
        }
    }

    let mut damaged: Vec<String> = report
        .segments
        .iter()
        .filter_map(|seg| match seg.kind {
            SegmentKind::Chunk {
                chunk_type,
                crc_ok: false,
                ..
            } => Some(format!(
                "CRC mismatch in {} at offset {}",
                String::from_utf8_lossy(&chunk_type),
                seg.range.start
            )),
            SegmentKind::Truncated => Some(format!(
                "truncated from offset {} ({} bytes)",
                seg.range.start,
                seg.range.len()
            )),
            SegmentKind::Trailer => Some(format!(
                "{} trailing bytes after IEND at offset {}",
                seg.range.len(),
                seg.range.start
            )),
            _ => None,
        })
        .collect();
    // A skip the file itself caused is a finding, and it is counted before the list is printed so
    // the exit message cannot report "0 finding(s)" while exiting non-zero. An over-budget skip is
    // not damage — nothing is known to be wrong with the file — so it is not one.
    if let FilterScan::Skipped(reason) = report.filters
        && reason.is_damage()
    {
        damaged.push(format!(
            "filters not counted — {}",
            filter_skip_label(reason)
        ));
    }
    print_lines("findings", &damaged);

    println!("  classified:    {}", yes_no(report.is_fully_classified()));
    println!("  intact:        {}", yes_no(report.is_intact()));

    if report.is_intact() {
        Ok(())
    } else {
        Err(CliError::NotFullyAccounted(format!(
            "{}: not a complete, undamaged PNG datastream — {} finding(s)",
            path.display(),
            damaged.len()
        )))
    }
}

/// Renders why a PNG's scanline filters were not counted.
fn filter_skip_label(reason: gamut::png::SkippedFilterScan) -> &'static str {
    use gamut::png::SkippedFilterScan as Reason;
    match reason {
        Reason::OverBudget => "the image is larger than the reader's byte budget",
        Reason::CorruptStream => "the IDAT stream is corrupt or truncated",
        Reason::LengthMismatch => "the IDAT stream inflated to the wrong length",
        Reason::UndefinedFilterCode => "a scanline carries an undefined filter code",
        // `SkippedFilterScan` is non-exhaustive; describe future reasons generically. They are
        // damage by default, so the finding is still raised.
        _ => "the scan could not be trusted",
    }
}

fn format_name(format: Format) -> &'static str {
    match format {
        Format::Tiff => "TIFF",
        Format::Dng => "DNG",
        Format::Png => "PNG",
    }
}

/// `yes`/`no` for a boolean verdict.
fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
