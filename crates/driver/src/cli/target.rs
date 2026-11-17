//! Target and language acceptance options.

use clap::Args;
use seal_target::TargetTriple;
use seal_version::LanguageVersion;

use super::values::{parse_language_version, parse_target};

/// The target and language version a compilation is checked against.
#[derive(Debug, Args)]
pub(crate) struct TargetArgs {
    /// Use this target triple instead of the host.
    #[arg(long, value_name = "TRIPLE", value_parser = parse_target, help_heading = "Target")]
    pub(crate) target: Option<TargetTriple>,

    /// Accept this language version instead of the current one.
    #[arg(
        long,
        value_name = "MAJOR.MINOR",
        value_parser = parse_language_version,
        help_heading = "Target"
    )]
    pub(crate) language_version: Option<LanguageVersion>,

    /// Accept unstable language features.
    #[arg(long = "allow-unstable-features", help_heading = "Target")]
    pub(crate) allow_unstable_features: bool,
}
