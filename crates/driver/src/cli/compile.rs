//! Options shared by every command that analyzes source files.

use std::path::PathBuf;

use clap::Args;

use super::diagnostics::DiagnosticArgs;
use super::target::TargetArgs;

/// The source files and the environment they are analyzed under.
#[derive(Debug, Args)]
pub(crate) struct CompileArgs {
    /// Source files to analyze.
    #[arg(value_name = "INPUT", required = true)]
    pub(crate) inputs: Vec<PathBuf>,

    #[command(flatten)]
    pub(crate) target: TargetArgs,

    #[command(flatten)]
    pub(crate) diagnostics: DiagnosticArgs,
}
