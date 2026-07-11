//! `gamut convert` — decode an image and re-encode it with a gamut codec.

use std::path::PathBuf;

use clap::{Args, ValueEnum};
use gamut::avif::AvifEncoder;
use gamut::core::{EncodeImage, ImageRef, Rgb8, Rgba8};
use gamut::jxl::{
    Container as JxlContainer, Distance as JxlDistance, Effort as JxlEffort, JxlEncoder,
};
use gamut::png::{Level as PngLevel, PngEncoder};
use gamut::tiff::{Compression as TiffCompression, TiffEncoder};
use gamut::webp::WebpEncoder;

use crate::error::CliError;
use crate::input::{decode_rgb8, decode_rgba8};

/// Arguments for `gamut convert`.
#[derive(Args)]
pub(crate) struct ConvertArgs {
    /// Input image (PNG, JPEG, PPM/P6, WebP, or JPEG XL). WebP and JPEG XL are decoded by gamut's
    /// own decoders.
    input: PathBuf,
    /// Output file. The format is inferred from its extension unless `--format` is given.
    output: PathBuf,
    /// Output format. Defaults to the output file's extension.
    #[arg(long, value_enum)]
    format: Option<OutputFormat>,
    /// AVIF mode selector: `0` keeps the lossless default; any nonzero value selects lossy AVIF at
    /// `--quality` (the encoder now takes a `0..=100` quality rather than a raw `base_q_idx`).
    #[arg(long, default_value_t = 0)]
    qindex: u8,
    /// Encode lossy (WebP VP8 intra) instead of lossless. For AVIF, select lossy with `--qindex`.
    #[arg(long)]
    lossy: bool,
    /// Lossy quality, 0–100 (higher is better but larger). Used with WebP `--lossy` and lossy AVIF.
    #[arg(long, default_value_t = 75)]
    quality: u8,
    /// Compress TIFF output with PackBits run-length encoding instead of storing it uncompressed.
    #[arg(long)]
    packbits: bool,
    /// JPEG XL Butteraugli distance for lossy encoding (~1.0 = visually lossless, up to 25.0).
    /// Supplying it selects lossy JXL; omitting it keeps the lossless default. Ignored for other
    /// output formats.
    #[arg(long)]
    jxl_distance: Option<f32>,
    /// JPEG XL encoder effort, 1 (fastest) to 10 (densest); libjxl's default is 7. Ignored for
    /// other output formats.
    #[arg(long, default_value_t = 7, value_parser = clap::value_parser!(u8).range(1..=10))]
    jxl_effort: u8,
    /// Emit JPEG XL in the ISO BMFF (`.jxl` box) container instead of a bare codestream. Ignored
    /// for other output formats.
    #[arg(long)]
    jxl_container: bool,
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
    /// PNG — lossless; transparency preserved, with automatic lossless colour-type reduction.
    Png,
    /// JPEG XL — lossless by default, or lossy at `--jxl-distance`; transparency preserved.
    Jxl,
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
            // `AvifEncoder` migrated from a raw `base_q_idx` to a lossless()/lossy(quality) model;
            // qindex 0 keeps the lossless default, any nonzero value selects lossy at --quality.
            let encoder = if args.qindex == 0 {
                AvifEncoder::lossless()
            } else {
                AvifEncoder::lossy(args.quality)
            };
            encoder.encode_image(ImageRef::<Rgb8>::new(&rgb, dims)?, &mut out)?;
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
        OutputFormat::Png => {
            // RGBA so transparency survives; auto-reduce drops it (and chooses grey/palette) when
            // that is lossless.
            let (rgba, dims) = decode_rgba8(&args.input)?;
            tracing::info!(
                width = dims.width,
                height = dims.height,
                bytes = rgba.len(),
                "decoded input"
            );
            PngEncoder::new()
                .with_compression(PngLevel::Best)
                .with_auto_reduce(true)
                .encode_image(ImageRef::<Rgba8>::new(&rgba, dims)?, &mut out)?;
            (rgba.len(), dims)
        }
        OutputFormat::Jxl => {
            // RGBA so transparency survives, matching the PNG/WebP paths.
            let (rgba, dims) = decode_rgba8(&args.input)?;
            tracing::info!(
                width = dims.width,
                height = dims.height,
                bytes = rgba.len(),
                "decoded input"
            );
            // A `--jxl-distance` selects lossy; `Distance::new` validates the range and surfaces an
            // out-of-range value as the codec's `InvalidInput` through `CliError::Codec`.
            let encoder = match args.jxl_distance {
                Some(distance) => JxlEncoder::lossy(JxlDistance::new(distance)?),
                None => JxlEncoder::lossless(),
            };
            // The clap `1..=10` range guarantees `from_level` returns `Some`; fall back to the
            // default effort (Squirrel/7) rather than unwrap so the path stays panic-free.
            let effort = JxlEffort::from_level(args.jxl_effort).unwrap_or_default();
            let container = if args.jxl_container {
                JxlContainer::IsoBmff
            } else {
                JxlContainer::Codestream
            };
            encoder
                .with_effort(effort)
                .with_container(container)
                .encode_image(ImageRef::<Rgba8>::new(&rgba, dims)?, &mut out)?;
            (rgba.len(), dims)
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
        Some("png") => Ok(OutputFormat::Png),
        Some("jxl") => Ok(OutputFormat::Jxl),
        Some(other) => Err(CliError::UnsupportedOutput(other.to_string())),
        None => Err(CliError::UnsupportedOutput("<none>".to_string())),
    }
}
