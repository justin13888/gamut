//! The CLI's error type, rendered to stderr by `main`.

use std::path::PathBuf;

/// Anything that can go wrong while running a `gamut` subcommand.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CliError {
    /// A filesystem read or write failed.
    #[error("i/o error on {path}: {source}")]
    Io {
        /// The file the operation targeted.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// An input image could not be decoded by the `image` crate.
    #[error("failed to decode image {path}: {source}")]
    Decode {
        /// The input file.
        path: PathBuf,
        /// The underlying decode error.
        #[source]
        source: image::ImageError,
    },

    /// A gamut codec rejected the input or hit an unsupported case.
    #[error(transparent)]
    Codec(#[from] gamut::core::Error),

    /// The requested output format is not (yet) supported by the CLI.
    #[error("unsupported output format: {0} (supported: 'avif', 'webp')")]
    UnsupportedOutput(String),

    /// A command argument was malformed in a way clap could not catch.
    #[error("invalid argument: {0}")]
    Usage(String),
}
