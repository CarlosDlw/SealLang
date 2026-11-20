//! Native link options shared by `build` and `run`.

use std::ffi::OsString;
use std::path::PathBuf;

use clap::Args;

/// Libraries, search paths, and raw linker arguments.
///
/// Every value is passed as one argument to the linker, never through a shell.
/// `--link-arg` accepts values that start with `-`, so flags such as
/// `--link-arg -Wl,--as-needed` reach the linker unchanged.
#[derive(Clone, Debug, Args)]
pub(crate) struct LinkArgs {
    /// Link against this native library (repeatable).
    #[arg(
        short = 'l',
        long = "library",
        value_name = "NAME",
        help_heading = "Link"
    )]
    pub(crate) libraries: Vec<String>,

    /// Add this directory to the linker search path (repeatable).
    #[arg(
        short = 'L',
        long = "library-path",
        value_name = "DIR",
        help_heading = "Link"
    )]
    pub(crate) search_paths: Vec<PathBuf>,

    /// Pass this argument to the linker unchanged (repeatable).
    #[arg(
        long = "link-arg",
        value_name = "ARG",
        allow_hyphen_values = true,
        help_heading = "Link"
    )]
    pub(crate) link_args: Vec<OsString>,
}
