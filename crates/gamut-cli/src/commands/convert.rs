//! `gamut convert` — decode an image and re-encode it with a gamut codec.

use std::path::PathBuf;

use clap::{Args, ValueEnum};
use gamut::avif::AvifEncoder;
use gamut::core::{EncodeImage, ImageRef, Rgb8, Rgba8};
use gamut::tiff::{Compression as TiffCompression, TiffEncoder};
use gamut::webp::WebpEncoder;

use crate::error::CliError;
use crate::input::{decode_rgb8, decode_rgba8};

/// Arguments for `gamut convert`.
#[derive(Args)]
pub(crate) struct ConvertArgs {
    /// Input image (PNG, JPEG, PPM/P6, or WebP). WebP is decoded by gamut's own decoder.
    input: PathBuf,
    /// Output file. The format is inferred from its extension unless `--format` is given.
    output: PathBuf,
    /// Output format. Defaults to the output file's extension.
    #[arg(long, value_enum)]
    format: Option<OutputFormat>,
    /// AV1 quantizer (`base_q_idx`): 0 is lossless (default), 1–255 is lossy intra (higher = more
    /// quantization, smaller files).
    #[arg(long, default_value_t = 0)]
    qindex: u8,
    /// Encode lossy (WebP VP8 intra) instead of lossless. Ignored for AVIF (use `--qindex`).
    #[arg(long)]
    lossy: bool,
    /// Lossy WebP quality, 0–100 (higher is better but larger). Only used with `--lossy`.
    #[arg(long, default_value_t = 75)]
    quality: u8,
    /// Compress TIFF output with PackBits run-length encoding instead of storing it uncompressed.
    #[arg(long)]
    packbits: bool,
}

/// Output container/codec for `gamut convert`.
#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum OutputFormat {
    /// AVIF (8-bit RGB; lossless or lossy intra via `--qindex`).
    Avif,
    /// WebP — lossless (VP8L) or lossy (VP8, with `--lossy`); transparency is preserved.
    Webp,
    /// TIFF (8-bit RGB; uncompressed, or PackBits with `--packbits`).
    Tiff,
}

/// Runs the `convert` command: decode the input, encode it, and report the result.
pub(crate) fn run(args: &ConvertArgs) -> Result<(), CliError> {
    let format = resolve_format(args)?;

    let mut out = Vec::new();
    let (raw_len, dims) = match format {
        OutputFormat::Avif => {
            let (rgb, dims) = decode_rgb8(&args.input)?;
            tracing::info!(
                width = dims.width,
                height = dims.height,
                bytes = rgb.len(),
                "decoded input"
            );
            AvifEncoder::new()
                .with_qindex(args.qindex)
                .encode_image(ImageRef::<Rgb8>::new(&rgb, dims)?, &mut out)?;
            (rgb.len(), dims)
        }
        OutputFormat::Webp => {
            // RGBA so transparency survives; `encode_rgba8` emits a simple file when fully opaque.
            let (rgba, dims) = decode_rgba8(&args.input)?;
            tracing::info!(
                width = dims.width,
                height = dims.height,
                bytes = rgba.len(),
                "decoded input"
            );
            let encoder = if args.lossy {
                WebpEncoder::lossy(args.quality)
            } else {
                WebpEncoder::lossless()
            };
            encoder.encode_image(ImageRef::<Rgba8>::new(&rgba, dims)?, &mut out)?;
            (rgba.len(), dims)
        }
        OutputFormat::Tiff => {
            let (rgb, dims) = decode_rgb8(&args.input)?;
            tracing::info!(
                width = dims.width,
                height = dims.height,
                bytes = rgb.len(),
                "decoded input"
            );
            let compression = if args.packbits {
                TiffCompression::PackBits
            } else {
                TiffCompression::None
            };
            let image = ImageRef::<Rgb8>::new(&rgb, dims)?;
            TiffEncoder::new()
                .with_compression(compression)
                .encode_image(image, &mut out)?;
            (rgb.len(), dims)
        }
    };
    tracing::info!(bytes = out.len(), lossy = args.lossy, "encoded output");

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
        Some("tiff" | "tif") => Ok(OutputFormat::Tiff),
        Some(other) => Err(CliError::UnsupportedOutput(other.to_string())),
        None => Err(CliError::UnsupportedOutput("<none>".to_string())),
    }
}
