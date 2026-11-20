//! The `run` subcommand.

use std::ffi::OsString;
use std::path::PathBuf;

use clap::Args;

use super::diagnostics::DiagnosticArgs;
use super::link::LinkArgs;
use super::optimization::OptimizationArgs;
use super::target::TargetArgs;

/// Builds a program into a temporary executable and runs it.
///
/// The executable lives only for the duration of the command. Arguments after
/// `--` are forwarded to the program unchanged.
#[derive(Debug, Args)]
#[command(after_help = "Examples:\\n  \
                        sealc run examples/006_structs.sl\\n  \
                        sealc run -O2 examples/006_structs.sl -- first second")]
pub(crate) struct RunArgs {
    /// Program to build and run.
    #[arg(value_name = "INPUT")]
    pub(crate) input: PathBuf,

    #[command(flatten)]
    pub(crate) target: TargetArgs,

    #[command(flatten)]
    pub(crate) diagnostics: DiagnosticArgs,

    #[command(flatten)]
    pub(crate) optimization: OptimizationArgs,

    #[command(flatten)]
    pub(crate) link: LinkArgs,

    /// Link executables with this runtime archive instead of the embedded one.
    #[arg(long, value_name = "PATH", help_heading = "Runtime")]
    pub(crate) runtime_archive: Option<PathBuf>,

    /// Arguments forwarded to the program.
    #[arg(last = true, value_name = "ARGS")]
    pub(crate) program_args: Vec<OsString>,
}
