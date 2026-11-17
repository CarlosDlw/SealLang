//! The reserved `run` subcommand.

use std::path::PathBuf;

use clap::Args;

/// Builds and runs a program.
///
/// This command is reserved and not implemented yet. The interface is kept
/// deliberately small so that adding execution later does not break callers.
#[derive(Debug, Args)]
pub(crate) struct RunArgs {
    /// Program to build and run.
    #[arg(value_name = "INPUT")]
    pub(crate) input: PathBuf,
}
