//! The `explain` subcommand.

use clap::Args;

/// Explains a diagnostic code.
#[derive(Debug, Args)]
#[command(after_help = "Examples:\n  sealc explain E5005")]
pub(crate) struct ExplainArgs {
    /// Diagnostic code to explain, for example `E5005`.
    #[arg(value_name = "CODE")]
    pub(crate) code: String,
}
