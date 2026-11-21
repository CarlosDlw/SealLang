//! The `sealc` command-line interface.
//!
//! The first argument selects a subcommand. Each subcommand owns only the
//! options it can honor, so `sealc check` cannot accept an artifact path and
//! `sealc build` cannot silently ignore a check-only request. Shared option
//! groups keep target selection and diagnostic presentation identical across
//! commands, and `clap` supplies help, version, suggestions, and consistent
//! usage errors.

mod build;
mod check;
mod codegen;
mod compile;
mod diagnostics;
mod emit_runtime;
mod explain;
mod link;
mod optimization;
mod output;
pub(crate) mod plan;
mod run;
mod target;
#[cfg(test)]
mod tests;
mod values;

use std::ffi::OsString;

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser, Subcommand};

pub(crate) use build::BuildArgs;
pub(crate) use check::CheckArgs;
pub(crate) use codegen::CodegenArgs;
pub(crate) use compile::CompileArgs;
pub(crate) use emit_runtime::EmitRuntimeArgs;
pub(crate) use explain::ExplainArgs;
pub(crate) use link::LinkArgs;
pub(crate) use output::OutputArgs;
pub(crate) use run::RunArgs;
pub(crate) use values::EmitArg;

/// Parsed command-line options.
#[derive(Debug, Parser)]
#[command(
    name = "sealc",
    version,
    about = "The Seal compiler",
    long_about = "The Seal compiler\n\n\
                  Builds, checks, and inspects Seal programs. Run `sealc help <COMMAND>`\n\
                  for the options of one command.\n\n\
                  Exit status: 0 on success, 1 after a compile error, 2 after a usage error,\n\
                  and 101 after an internal compiler error or a runtime panic. `sealc run`\n\
                  returns the exit status of the program it executes.",
    subcommand_required = true,
    arg_required_else_help = true,
    after_help = "Examples:\n  \
                  sealc build examples/006_structs.sl\n  \
                  sealc run examples/006_structs.sl\n  \
                  sealc check examples/006_structs.sl\n  \
                  sealc build --emit=obj -O2 examples/006_structs.sl -o /tmp/structs.o\n  \
                  sealc explain E5005\n  \
                  sealc emit-runtime /tmp/seal_rt.a"
)]
pub(crate) struct Cli {
    /// The action to perform.
    #[command(subcommand)]
    pub(crate) command: CliCommand,
}

/// The available subcommands.
#[derive(Debug, Subcommand)]
pub(crate) enum CliCommand {
    /// Compile inputs into executables, objects, or assembly.
    Build(BuildArgs),
    /// Build a program into a temporary executable and run it.
    Run(RunArgs),
    /// Check inputs without writing an artifact.
    Check(CheckArgs),
    /// Generate one object from a `.sealmono` unit.
    Codegen(CodegenArgs),
    /// Explain a diagnostic code.
    Explain(ExplainArgs),
    /// Write the embedded runtime archive to a path.
    EmitRuntime(EmitRuntimeArgs),
    /// Print compiler version information.
    Version,
}

impl Cli {
    /// Parses `argv`, including the program name, and validates the request.
    pub(crate) fn parse_from<I, T>(argv: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let cli = Self::try_parse_from(argv)?;
        cli.validate()
    }

    /// Applies the rules that cannot be expressed as per-argument constraints.
    fn validate(self) -> Result<Self, clap::Error> {
        let compile = match &self.command {
            CliCommand::Build(build) => Some(&build.compile),
            CliCommand::Check(check) => Some(&check.compile),
            _ => None,
        };
        if let Some(compile) = compile {
            if compile.plan_file.is_some() && !compile.inputs.is_empty() {
                return Err(Self::command().error(
                    ErrorKind::ArgumentConflict,
                    "`--plan-file` cannot be combined with input files",
                ));
            }
            if compile.plan_file.is_some() && compile.plan {
                return Err(Self::command().error(
                    ErrorKind::ArgumentConflict,
                    "`--plan-file` and `--plan` cannot be combined",
                ));
            }
            if compile.plan_file.is_none() && compile.inputs.is_empty() {
                return Err(Self::command().error(
                    ErrorKind::MissingRequiredArgument,
                    "provide at least one input file or `--plan-file`",
                ));
            }
            if compile.plan_file.is_some() && !compile.expose.is_empty() {
                return Err(Self::command().error(
                    ErrorKind::ArgumentConflict,
                    "`--expose` belongs to `--plan`; a plan names its own exposed modules",
                ));
            }
        }
        if let CliCommand::Build(build) = &self.command {
            if build.output.path.is_some() && build.compile.inputs.len() > 1 && !build.compile.plan
            {
                return Err(Self::command().error(
                    ErrorKind::ArgumentConflict,
                    "`--output` can only be used with a single input file or `--plan`",
                ));
            }
        }
        Ok(self)
    }
}
