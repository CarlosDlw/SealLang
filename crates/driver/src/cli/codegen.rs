//! The `codegen` subcommand: one `.sealmono` unit to one object.

use std::path::PathBuf;

use clap::Args;

/// Generates one native object from a frontend metadata unit.
#[derive(Debug, Args)]
#[command(after_help = "Examples:\n  \
                        sealc codegen --mono app.sealmono --cgu 0 -o app.0.o\n  \
                        sealc codegen --mono app.sealmono --cgu 2 -o app.2.o")]
pub(crate) struct CodegenArgs {
    /// The `.sealmono` artifact written by `sealc build --emit=mono`.
    #[arg(long, value_name = "PATH", help_heading = "Input")]
    pub(crate) mono: PathBuf,

    /// The zero-based code generation unit to emit.
    #[arg(long, value_name = "N", default_value_t = 0, help_heading = "Input")]
    pub(crate) cgu: u32,

    /// Write the object here.
    #[arg(
        short = 'o',
        long = "output",
        value_name = "PATH",
        help_heading = "Output"
    )]
    pub(crate) output: Option<PathBuf>,
}
