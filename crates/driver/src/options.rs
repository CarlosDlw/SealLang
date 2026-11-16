//! Translation from parsed arguments to session options.
//!
//! Every phase reads one [`seal_session::Options`] value; this module is the
//! only place that decides how a flag maps onto it.

use seal_diagnostics::DEFAULT_ERROR_LIMIT;
use seal_session::Options;
use seal_target::host_triple;
use seal_version::LanguageVersion;

use crate::cli::Cli;

/// Builds the options every phase reads for this invocation.
pub(crate) fn session_options(cli: &Cli) -> Options {
    Options {
        target: cli.target.target.clone().unwrap_or_else(host_triple),
        opt_level: cli.output.opt_level,
        debug_info: cli.output.debug_info,
        language_version: cli
            .target
            .language_version
            .unwrap_or(LanguageVersion::CURRENT),
        allow_unstable: cli.target.allow_unstable_features,
        remap_prefixes: cli.diagnostics.remap_path_prefix.clone(),
        max_errors: cli.diagnostics.max_errors.unwrap_or(DEFAULT_ERROR_LIMIT),
        error_format: cli.diagnostics.error_format.into(),
        color: cli.diagnostics.color.into(),
        time_passes: cli.diagnostics.time_passes,
        ..Options::default()
    }
}
