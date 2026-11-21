//! Diagnostic presentation options.

use clap::Args;

use super::values::{ColorArg, ErrorFormatArg, parse_positive};

/// How diagnostics are rendered and limited.
#[derive(Clone, Debug, Args)]
pub(crate) struct DiagnosticArgs {
    /// Diagnostic format.
    #[arg(
        long,
        value_name = "FORMAT",
        value_enum,
        default_value = "human",
        help_heading = "Diagnostics"
    )]
    pub(crate) error_format: ErrorFormatArg,

    /// When to color diagnostics.
    #[arg(
        long,
        value_name = "WHEN",
        value_enum,
        default_value = "auto",
        help_heading = "Diagnostics"
    )]
    pub(crate) color: ColorArg,

    /// Stop emitting after this many errors.
    #[arg(long, value_name = "N", value_parser = parse_positive, help_heading = "Diagnostics")]
    pub(crate) max_errors: Option<usize>,

    /// Rewrite a prefix in diagnostic paths; repeatable.
    #[arg(
        long = "remap-path-prefix",
        value_name = "FROM=TO",
        help_heading = "Diagnostics"
    )]
    pub(crate) remap_path_prefix: Vec<String>,

    /// Print how long each compiler phase took.
    #[arg(long = "time-passes", help_heading = "Diagnostics")]
    pub(crate) time_passes: bool,

    /// Treat every emitted warning as an error.
    #[arg(long = "deny-warnings", help_heading = "Diagnostics")]
    pub(crate) deny_warnings: bool,
}
