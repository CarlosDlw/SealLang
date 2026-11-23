//! Translation from parsed arguments to session options.
//!
//! Every phase reads one [`seal_session::Options`] value; this module is the
//! only place that decides how a flag maps onto it.

use seal_diagnostics::{DEFAULT_ERROR_LIMIT, KNOWN_LINTS, known_lint};
use seal_session::{OptLevel, Options};
use seal_target::host_triple;
use seal_version::{Edition, LanguageVersion};

use crate::cli::{CompileArgs, OutputArgs};

/// Builds the options every phase reads for this invocation.
///
/// `output` is present only for commands that can write an artifact; checking
/// has no optimization or debug-information choice and therefore keeps the
/// compiler defaults.
pub(crate) fn session_options(arguments: &CompileArgs, output: Option<&OutputArgs>) -> Options {
    Options {
        target: arguments.target.target.clone().unwrap_or_else(host_triple),
        opt_level: output.map_or(OptLevel::O0, |output| output.optimization.opt_level),
        debug_info: output.is_some_and(|output| output.optimization.debug_info),
        language_version: arguments
            .target
            .language_version
            .unwrap_or(LanguageVersion::CURRENT),
        edition: arguments.target.edition.unwrap_or(Edition::CURRENT),
        allow_unstable: arguments.target.allow_unstable_features,
        allow_lints: arguments.target.allow_lints.clone(),
        deny_lints: arguments.target.deny_lints.clone(),
        remap_prefixes: arguments.diagnostics.remap_path_prefix.clone(),
        max_errors: arguments
            .diagnostics
            .max_errors
            .unwrap_or(DEFAULT_ERROR_LIMIT),
        error_format: arguments.diagnostics.error_format.into(),
        color: arguments.diagnostics.color.into(),
        time_passes: arguments.diagnostics.time_passes,
        ..Options::default()
    }
}

/// Rejects a lint flag that does not name a compiler lint.
///
/// The `warnings` group belongs to `seal lint`; the driver's per-lint flags
/// accept only individual names. A name in both lists is contradictory and is
/// rejected before any compilation work starts.
pub(crate) fn validate_lints(arguments: &CompileArgs) -> Result<(), String> {
    for name in arguments
        .target
        .allow_lints
        .iter()
        .chain(&arguments.target.deny_lints)
    {
        if known_lint(name).is_none() {
            return Err(format!(
                "unknown lint `{name}`; the compiler lints are {}",
                KNOWN_LINTS.join(", ")
            ));
        }
    }
    for name in &arguments.target.deny_lints {
        if arguments.target.allow_lints.contains(name) {
            return Err(format!("the lint `{name}` is both allowed and denied"));
        }
    }
    Ok(())
}
