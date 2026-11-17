//! The `emit-runtime` subcommand.

use std::path::PathBuf;

use clap::Args;

/// Writes the embedded runtime archive to a path.
#[derive(Debug, Args)]
#[command(after_help = "Examples:\n  sealc emit-runtime /tmp/seal_rt.a")]
pub(crate) struct EmitRuntimeArgs {
    /// Destination of the runtime archive.
    #[arg(value_name = "PATH")]
    pub(crate) path: PathBuf,
}
