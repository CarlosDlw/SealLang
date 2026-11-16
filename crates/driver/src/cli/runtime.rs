//! Runtime archive options.

use std::path::PathBuf;

use clap::Args;

/// How the native runtime archive is exported or selected.
#[derive(Debug, Args)]
pub(crate) struct RuntimeArgs {
    /// Write the embedded host runtime archive here and exit.
    #[arg(long, value_name = "PATH", help_heading = "Runtime")]
    pub(crate) emit_runtime: Option<PathBuf>,

    /// Link executables with this runtime archive instead of the embedded one.
    #[arg(long, value_name = "PATH", help_heading = "Runtime")]
    pub(crate) runtime_archive: Option<PathBuf>,
}
