//! `gamut` — a command-line sandbox over the gamut image codecs and their shared primitives.
//!
//! It decodes PNG/JPEG/PPM input (via the third-party [`image`] crate) and encodes exclusively
//! with the gamut crates, and surfaces the shared `color` / `dsp` / `bitstream` primitives as
//! inspection subcommands so the latest workspace features are exercisable without writing
//! throwaway Rust. See the crate README for the full command reference.
#![forbid(unsafe_code)]

mod commands;
mod error;
mod input;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// Detailed version string for `-V`/`--version`: the package version plus build provenance
/// (git commit, working-tree state, build profile, target triple, rustc, and commit date),
/// all captured at compile time by `build.rs`. Useful for pinning down exactly which build a
/// bug report came from.
const LONG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "\ncommit:  ",
    env!("GAMUT_GIT_HASH"),
    " (",
    env!("GAMUT_GIT_DIRTY"),
    ")",
    "\nprofile: ",
    env!("GAMUT_BUILD_PROFILE"),
    "\ntarget:  ",
    env!("GAMUT_BUILD_TARGET"),
    "\nrustc:   ",
    env!("GAMUT_RUSTC_VERSION"),
    "\ncommit date: ",
    env!("GAMUT_COMMIT_DATE"),
);

/// Sandbox CLI for the gamut codecs and primitives.
#[derive(Parser)]
#[command(name = "gamut", version = LONG_VERSION, about)]
struct Cli {
    /// Increase log verbosity (`-v` = info, `-vv` = debug). `RUST_LOG` overrides this.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

/// Top-level subcommands, each grouped by the crate it exercises.
#[derive(Subcommand)]
enum Command {
    /// Decode an image (PNG/JPEG/PPM) and re-encode it with a gamut codec (gamut-avif).
    Convert(commands::convert::ConvertArgs),
    /// AV1 still-image operations (gamut-av1).
    #[command(subcommand)]
    Av1(commands::av1::Av1Command),
    /// Inspect color tables: CICP code points and pixel formats (gamut-color).
    #[command(subcommand)]
    Color(commands::color::ColorCommand),
    /// Run the Walsh–Hadamard transform (gamut-dsp).
    #[command(subcommand)]
    Dsp(commands::dsp::DspCommand),
    /// Exercise bitstream primitives (gamut-bitstream).
    #[command(subcommand)]
    Bitstream(commands::bitstream::BitstreamCommand),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    let result = match cli.command {
        Command::Convert(args) => commands::convert::run(&args),
        Command::Av1(cmd) => commands::av1::run(&cmd),
        Command::Color(cmd) => commands::color::run(&cmd),
        Command::Dsp(cmd) => commands::dsp::run(&cmd),
        Command::Bitstream(cmd) => commands::bitstream::run(&cmd),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Initializes a stderr `tracing` subscriber. `RUST_LOG` takes precedence; otherwise the
/// `-v`/`--verbose` count maps to a level (0 = warn, 1 = info, 2+ = debug). Logs go to stderr so
/// stdout stays clean for command output.
fn init_tracing(verbose: u8) {
    use tracing_subscriber::EnvFilter;

    let filter = match std::env::var("RUST_LOG") {
        Ok(directives) if !directives.is_empty() => EnvFilter::new(directives),
        _ => EnvFilter::new(match verbose {
            0 => "warn",
            1 => "info",
            _ => "debug",
        }),
    };

    // `try_init` fails only if a subscriber is already set; nothing to recover, so ignore it.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}
