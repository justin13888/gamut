//! `gamut dsp` — the lossless 4×4 Walsh–Hadamard transform (gamut-dsp).

use clap::Subcommand;
use gamut::dsp::{fwht4x4, iwht4x4};

use crate::error::CliError;

/// `gamut dsp` subcommands.
#[derive(Subcommand)]
pub(crate) enum DspCommand {
    /// Apply the forward 4×4 WHT to 16 integers and verify the inverse round-trip.
    Wht {
        /// Exactly 16 integer coefficients (row-major 4×4 block). Put `--inverse` first if used.
        #[arg(num_args = 16, allow_hyphen_values = true)]
        values: Vec<i32>,
        /// Run the inverse transform first (treat the input as coefficients).
        #[arg(long)]
        inverse: bool,
    },
}

/// Runs a `dsp` subcommand.
pub(crate) fn run(cmd: &DspCommand) -> Result<(), CliError> {
    match cmd {
        DspCommand::Wht { values, inverse } => wht(values, *inverse),
    }
}

/// Transforms the 4×4 block and prints the result plus the round-trip back to the input.
fn wht(values: &[i32], inverse: bool) -> Result<(), CliError> {
    let block: [i32; 16] = values.try_into().map_err(|_| {
        CliError::Usage(format!("expected exactly 16 values, got {}", values.len()))
    })?;

    let (transformed, roundtrip) = if inverse {
        let coeffs = iwht4x4(&block);
        (coeffs, fwht4x4(&coeffs))
    } else {
        let coeffs = fwht4x4(&block);
        (coeffs, iwht4x4(&coeffs))
    };

    let label = if inverse { "inverse" } else { "forward" };
    println!("input:       {block:?}");
    println!("{label:>7}:    {transformed:?}");
    println!("round-trip:  {roundtrip:?}");
    println!(
        "round-trip:  {}",
        if roundtrip == block {
            "matches input"
        } else {
            "DIFFERS from input"
        }
    );
    Ok(())
}
