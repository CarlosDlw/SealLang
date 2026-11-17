//! The `build` subcommand.

use std::path::PathBuf;

use clap::Args;

use super::compile::CompileArgs;
use super::output::OutputArgs;

/// Compiles inputs into executables, objects, or assembly.
#[derive(Debug, Args)]
#[command(after_help = "Examples:\n  \
                        sealc build examples/006_structs.sl\n  \
                        sealc build --emit=obj -O2 examples/006_structs.sl -o /tmp/structs.o")]
pub(crate) struct BuildArgs {
    #[command(flatten)]
    pub(crate) output: OutputArgs,

    #[command(flatten)]
    pub(crate) compile: CompileArgs,

    /// Link executables with this runtime archive instead of the embedded one.
    #[arg(long, value_name = "PATH", help_heading = "Runtime")]
    pub(crate) runtime_archive: Option<PathBuf>,
}
