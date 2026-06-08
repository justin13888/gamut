//! `gamut bitstream` — exercise the bitstream primitives (gamut-bitstream).

use clap::Subcommand;
use gamut::bitstream::{leb128_len, write_leb128};

use crate::error::CliError;

/// `gamut bitstream` subcommands.
#[derive(Subcommand)]
pub(crate) enum BitstreamCommand {
    /// Show the unsigned LEB128 encoding of a value (hex bytes + byte length).
    Leb128 {
        /// The unsigned value to encode.
        value: u64,
    },
}

/// Runs a `bitstream` subcommand.
pub(crate) fn run(cmd: &BitstreamCommand) -> Result<(), CliError> {
    match cmd {
        BitstreamCommand::Leb128 { value } => {
            leb128(*value);
            Ok(())
        }
    }
}

/// Prints the LEB128 byte encoding of `value` and its precomputed length.
fn leb128(value: u64) {
    let mut buf = Vec::new();
    write_leb128(&mut buf, value);
    let hex = buf
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    println!("value:  {value}");
    println!("leb128: {hex} ({} bytes)", buf.len());
    println!("len():  {} (matches)", leb128_len(value));
}
