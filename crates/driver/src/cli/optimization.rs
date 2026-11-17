//! Optimization and debug-information options shared by build and run.

use clap::Args;
use seal_session::OptLevel;

use super::values::parse_opt_level;

/// How the generated code is optimized and described.
#[derive(Clone, Debug, Args)]
pub(crate) struct OptimizationArgs {
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
