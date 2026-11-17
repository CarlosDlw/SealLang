//! Artifact and optimization options.

use std::path::PathBuf;

use clap::Args;
use seal_codegen::Emit;

use super::optimization::OptimizationArgs;
use super::values::EmitArg;

/// What to produce and how to optimize it.
#[derive(Debug, Args)]
pub(crate) struct OutputArgs {
    /// Artifact to produce.
    #[arg(
        long,
        value_name = "KIND",
        value_enum,
        default_value = "exe",
        help_heading = "Output"
    )]
    pub(crate) emit: EmitArg,

    /// Write the artifact here instead of next to the input.
    #[arg(
        short = 'o',
        long = "output",
        value_name = "PATH",
        help_heading = "Output"
    )]
    pub(crate) path: Option<PathBuf>,

    #[command(flatten)]
    pub(crate) optimization: OptimizationArgs,
}

impl OutputArgs {
    /// Returns the artifact a successful build should write.
    pub(crate) fn artifact(&self) -> Emit {
        self.emit.into()
    }
}
