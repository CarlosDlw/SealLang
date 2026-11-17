//! The `check` subcommand.

use clap::Args;

use super::compile::CompileArgs;

/// Checks inputs without writing an artifact.
#[derive(Debug, Args)]
#[command(
    after_help = "Examples:\n  sealc check examples/006_structs.sl\n  sealc check --target=aarch64-unknown-linux-gnu main.sl"
)]
pub(crate) struct CheckArgs {
    #[command(flatten)]
    pub(crate) compile: CompileArgs,
}
