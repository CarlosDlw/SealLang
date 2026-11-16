//! Artifact and optimization options.

use std::path::PathBuf;

use clap::Args;
use seal_session::OptLevel;

use super::values::{EmitArg, RequestedOutput, parse_opt_level};

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

    /// Emit DWARF debug information.
    #[arg(short = 'g', long = "debug-info", help_heading = "Output")]
    pub(crate) debug_info: bool,

    /// Optimization level; `-O` alone selects 3.
    #[arg(
        short = 'O',
        long = "opt-level",
        value_name = "LEVEL",
        num_args = 0..=1,
        default_value = "0",
        default_missing_value = "3",
        value_parser = parse_opt_level,
        help_heading = "Optimization"
    )]
    pub(crate) opt_level: OptLevel,
}

impl OutputArgs {
    /// Returns what a successful compilation should write.
    pub(crate) fn requested(&self) -> RequestedOutput {
        self.emit.into()
    }
}
