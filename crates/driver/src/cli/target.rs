//! Target and language acceptance options.

use clap::Args;
use seal_target::TargetTriple;
use seal_version::{Edition, LanguageVersion};

use super::values::{parse_edition, parse_language_version, parse_target};

/// The target and language version a compilation is checked against.
#[derive(Clone, Debug, Args)]
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

    /// Compile as this compatibility edition instead of the current one.
    #[arg(
        long,
        value_name = "YEAR",
        value_parser = parse_edition,
        help_heading = "Target"
    )]
    pub(crate) edition: Option<Edition>,

    /// Accept unstable language features.
    #[arg(long = "allow-unstable-features", help_heading = "Target")]
    pub(crate) allow_unstable_features: bool,
}
