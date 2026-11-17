//! Translation from parsed arguments to session options.
//!
//! Every phase reads one [`seal_session::Options`] value; this module is the
//! only place that decides how a flag maps onto it.

use seal_diagnostics::DEFAULT_ERROR_LIMIT;
use seal_session::{OptLevel, Options};
use seal_target::host_triple;
use seal_version::LanguageVersion;

use crate::cli::{CompileArgs, OutputArgs};

/// Builds the options every phase reads for this invocation.
///
/// `output` is present only for commands that can write an artifact; checking
/// has no optimization or debug-information choice and therefore keeps the
/// compiler defaults.
pub(crate) fn session_options(arguments: &CompileArgs, output: Option<&OutputArgs>) -> Options {
    Options {
        target: arguments.target.target.clone().unwrap_or_else(host_triple),
        opt_level: output.map_or(OptLevel::O0, |output| output.opt_level),
        debug_info: output.is_some_and(|output| output.debug_info),
        language_version: arguments
            .target
            .language_version
            .unwrap_or(LanguageVersion::CURRENT),
        allow_unstable: arguments.target.allow_unstable_features,
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
