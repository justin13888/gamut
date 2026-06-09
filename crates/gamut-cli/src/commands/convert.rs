//! `gamut convert` — decode an image and re-encode it with a gamut codec.

use std::path::PathBuf;

use clap::{Args, ValueEnum};
use gamut::avif::AvifEncoder;
use gamut::webp::WebpEncoder;

use crate::error::CliError;
use crate::input::decode_rgb8;

/// Arguments for `gamut convert`.
#[derive(Args)]
pub(crate) struct ConvertArgs {
    /// Input image (PNG, JPEG, or PPM/P6).
    input: PathBuf,
    /// Output file. The format is inferred from its extension unless `--format` is given.
    output: PathBuf,
    /// Output format. Defaults to the output file's extension.
    #[arg(long, value_enum)]
    format: Option<OutputFormat>,
}

/// Output container/codec for `gamut convert`. AVIF encoding is implemented; WebP is recognized but
/// its encoder is still under construction (it returns an "unsupported" error until VP8L M0 lands).
#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum OutputFormat {
    /// AVIF (lossless 8-bit RGB, milestone M0).
    Avif,
    /// WebP (VP8L lossless; encoder not yet implemented).
    Webp,
}

/// Runs the `convert` command: decode the input, encode it, and report the result.
pub(crate) fn run(args: &ConvertArgs) -> Result<(), CliError> {
    let format = resolve_format(args)?;

    let (rgb, dims) = decode_rgb8(&args.input)?;
    let raw_len = rgb.len();
    tracing::info!(
        width = dims.width,
        height = dims.height,
        bytes = raw_len,
        "decoded input"
    );

    let mut out = Vec::new();
    match format {
        OutputFormat::Avif => {
            AvifEncoder::new().encode_rgb8(&rgb, dims, &mut out)?;
        }
        OutputFormat::Webp => {
            WebpEncoder::new().encode_rgb8(&rgb, dims, &mut out)?;
        }
    }
    tracing::info!(bytes = out.len(), "encoded output");

    std::fs::write(&args.output, &out).map_err(|source| CliError::Io {
        path: args.output.clone(),
        source,
    })?;

    let ratio = if out.is_empty() {
        0.0
    } else {
        raw_len as f64 / out.len() as f64
    };
    println!(
        "wrote {} ({}x{}, {} bytes, {ratio:.2}x vs raw RGB)",
        args.output.display(),
        dims.width,
        dims.height,
        out.len(),
    );
    Ok(())
}

/// Picks the output format from `--format`, falling back to the output file's extension.
fn resolve_format(args: &ConvertArgs) -> Result<OutputFormat, CliError> {
    if let Some(format) = args.format {
        return Ok(format);
    }
    match args
        .output
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("avif") => Ok(OutputFormat::Avif),
        Some("webp") => Ok(OutputFormat::Webp),
        Some(other) => Err(CliError::UnsupportedOutput(other.to_string())),
        None => Err(CliError::UnsupportedOutput("<none>".to_string())),
    }
}
