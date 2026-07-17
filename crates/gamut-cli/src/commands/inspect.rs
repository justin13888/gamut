//! `gamut inspect` — strict "deconstruct" of a TIFF or DNG (issue #197).
//!
//! Walks the entire container, accounts every byte, and flags anything unrecognised (unknown
//! tags, unknown field types, out-of-spec codes, unaccounted bytes). Prints a report to stdout and
//! exits non-zero when the file is not fully accounted for — usable as an archival CI gate.

use std::path::PathBuf;

use clap::{Args, ValueEnum};
use gamut::ifd::{CoverageReport, UnknownField};

use crate::error::CliError;

/// The most list entries to print per category before truncating.
const MAX_LIST: usize = 20;

/// The `DNGVersion` tag (50706) — present in every DNG, absent in plain TIFF; the format sniff.
const DNG_VERSION_TAG: u16 = 50706;

/// Arguments for `gamut inspect`.
#[derive(Args)]
pub(crate) struct InspectArgs {
    /// Input TIFF or DNG file.
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
}

/// A format-agnostic view of a deconstruct report, for printing.
struct Summary {
    coverage: CoverageReport,
    unknown_fields: Vec<UnknownField>,
    unknown_tags: Vec<String>,
    anomalies: Vec<String>,
    fully_covered: bool,
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

    let summary = match format {
        Format::Dng => summarize_dng(gamut::dng::deconstruct(&data)?),
        Format::Tiff => summarize_tiff(gamut::tiff::deconstruct(&data)?),
    };

    print_summary(&args.input, format, &summary);

    if summary.fully_accounted {
        Ok(())
    } else {
        Err(CliError::NotFullyAccounted(format!(
            "{}: not fully accounted — {} unaccounted byte(s), {} unknown tag(s), {} unknown field type(s), {} anomaly/ies",
            args.input.display(),
            summary.coverage.unaccounted_bytes(),
            summary.unknown_tags.len(),
            summary.unknown_fields.len(),
            summary.anomalies.len(),
        )))
    }
}

/// Detects DNG vs TIFF: a DNG is a TIFF whose IFD 0 carries the mandatory `DNGVersion` tag.
fn sniff(data: &[u8]) -> Format {
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
    let fully_covered = report.is_fully_covered();
    let fully_accounted = report.is_fully_accounted();
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
        coverage: report.coverage,
        unknown_fields: report.unknown_fields,
        unknown_tags,
        anomalies,
        fully_covered,
        fully_accounted,
    }
}

/// Collapses a DNG report into the printable [`Summary`].
fn summarize_dng(report: gamut::dng::DeconstructReport) -> Summary {
    use gamut::dng::Anomaly;
    let fully_covered = report.is_fully_covered();
    let fully_accounted = report.is_fully_accounted();
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
            } => {
                format!("[error] page {page}: {detail} (tag {tag:#06x}, code {code})")
            }
            Anomaly::UnparsableTag { page, tag, detail } => {
                format!("[error] page {page}: {detail} (tag {tag:#06x})")
            }
            Anomaly::Structure {
                page,
                detail,
                severity,
            } => {
                format!("[{}] page {page}: {detail}", severity_label_dng(*severity))
            }
        })
        .collect();
    Summary {
        coverage: report.coverage,
        unknown_fields: report.unknown_fields,
        unknown_tags,
        anomalies,
        fully_covered,
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
    }
}

/// Prints the human-readable report to stdout.
fn print_summary(path: &std::path::Path, format: Format, s: &Summary) {
    let cov = &s.coverage;
    let pct = if cov.file_len == 0 {
        100.0
    } else {
        cov.covered_bytes as f64 / cov.file_len as f64 * 100.0
    };
    println!("inspecting {} as {}", path.display(), format_name(format));
    println!(
        "  covered:      {} / {} bytes ({pct:.1}%)",
        cov.covered_bytes, cov.file_len
    );
    println!("  unaccounted:  {} bytes", cov.unaccounted_bytes());

    if let Some(t) = &cov.trailing {
        println!("  trailing:     {} bytes at offset {}", t.len, t.start);
    }
    print_ranges(
        "gaps",
        &cov.gaps
            .iter()
            .map(|g| (g.start, g.len))
            .collect::<Vec<_>>(),
    );
    if !cov.overlaps.is_empty() {
        println!("  overlaps:     {}", cov.overlaps.len());
        for o in cov.overlaps.iter().take(MAX_LIST) {
            println!(
                "    - [{}, {}) overlaps [{}, {})",
                o.b.start,
                o.b.end(),
                o.a.start,
                o.a.end()
            );
        }
    }
    print_ranges(
        "out-of-bounds",
        &cov.out_of_bounds
            .iter()
            .map(|r| (r.start, r.len))
            .collect::<Vec<_>>(),
    );

    print_lines(
        "unknown field types",
        &s.unknown_fields
            .iter()
            .map(|u| {
                format!(
                    "tag {:#06x} type {} (count {}) at ifd offset {}",
                    u.tag, u.type_code, u.count, u.ifd_offset
                )
            })
            .collect::<Vec<_>>(),
    );
    print_lines("unknown tags", &s.unknown_tags);
    print_lines("anomalies", &s.anomalies);

    println!("  fully covered:    {}", yes_no(s.fully_covered));
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
fn format_name(format: Format) -> &'static str {
    match format {
        Format::Tiff => "TIFF",
        Format::Dng => "DNG",
    }
}

/// `yes`/`no` for a boolean verdict.
fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
