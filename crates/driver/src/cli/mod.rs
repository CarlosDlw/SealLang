//! The `sealc` command-line interface.
//!
//! Parsing is declarative: each option group owns one concern, and `clap`
//! supplies help, version, suggestions, and consistent usage errors. Rules that
//! span arguments (a required input, `-o` with several inputs) are applied after
//! parsing and returned as ordinary `clap` errors, so every usage failure has
//! the same shape and exit status.

mod diagnostics;
mod output;
mod runtime;
mod target;
mod values;

use std::ffi::OsString;
use std::path::PathBuf;

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser};

pub(crate) use diagnostics::DiagnosticArgs;
pub(crate) use output::OutputArgs;
pub(crate) use runtime::RuntimeArgs;
pub(crate) use target::TargetArgs;
pub(crate) use values::RequestedOutput;

/// Parsed command-line options.
#[derive(Debug, Parser)]
#[command(
    name = "sealc",
    version,
    about = "The Seal compiler",
    override_usage = "sealc [OPTIONS] <INPUT>...\n       sealc --emit-runtime <PATH>\n       sealc --explain <CODE>",
    long_about = "The Seal compiler\n\n\
                  Compiles Seal source files, checks them, and links native executables.\n\n\
                  Exit status: 0 on success, 1 after a compile error, 2 after a usage error,\n\
                  and 101 after an internal compiler error or a runtime panic."
)]
pub(crate) struct Cli {
    /// Source files to compile.
    #[arg(
        value_name = "INPUT",
        required_unless_present_any = ["emit_runtime", "explain"]
    )]
    pub(crate) inputs: Vec<PathBuf>,

    #[command(flatten)]
    pub(crate) output: OutputArgs,

    #[command(flatten)]
    pub(crate) target: TargetArgs,

    #[command(flatten)]
    pub(crate) diagnostics: DiagnosticArgs,

    #[command(flatten)]
    pub(crate) runtime: RuntimeArgs,

    /// Explain a diagnostic code and exit.
    #[arg(long, value_name = "CODE", help_heading = "Meta")]
    pub(crate) explain: Option<String>,
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
        if self.output.path.is_some() && self.inputs.len() > 1 {
            return Err(Self::command().error(
                ErrorKind::ArgumentConflict,
                "`--output` can only be used with a single input file",
            ));
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use clap::CommandFactory;
    use clap::error::ErrorKind;
    use seal_codegen::Emit;
    use seal_session::{ErrorFormat, OptLevel};
    use seal_target::TargetTriple;

    use super::{Cli, RequestedOutput};

    fn parse(arguments: &[&str]) -> Result<Cli, clap::Error> {
        Cli::parse_from(std::iter::once("sealc").chain(arguments.iter().copied()))
    }

    fn parsed(arguments: &[&str]) -> Cli {
        parse(arguments).unwrap_or_else(|error| panic!("arguments did not parse: {error}"))
    }

    #[test]
    fn defaults_match_the_compiler_defaults() {
        let cli = parsed(&["main.sl"]);
        assert_eq!(cli.inputs, vec![PathBuf::from("main.sl")]);
        assert_eq!(cli.output.emit, super::values::EmitArg::Exe);
        assert_eq!(cli.output.opt_level, OptLevel::O0);
        assert!(!cli.output.debug_info);
        assert_eq!(
            cli.diagnostics.error_format,
            super::values::ErrorFormatArg::Human
        );
        assert_eq!(
            ErrorFormat::from(cli.diagnostics.error_format),
            ErrorFormat::Human
        );
        assert!(cli.target.target.is_none());
        assert!(cli.target.language_version.is_none());
        assert!(!cli.target.allow_unstable_features);
        assert!(cli.runtime.runtime_archive.is_none());
        assert!(cli.runtime.emit_runtime.is_none());
        assert!(cli.explain.is_none());
        assert_eq!(
            cli.output.requested(),
            RequestedOutput::Artifact(Emit::Executable)
        );
    }

    #[test]
    fn optimization_levels_accept_attached_separate_and_bare_forms() {
        assert_eq!(parsed(&["main.sl", "-O"]).output.opt_level, OptLevel::O3);
        assert_eq!(parsed(&["-O2", "main.sl"]).output.opt_level, OptLevel::O2);
        assert_eq!(
            parsed(&["--opt-level=1", "main.sl"]).output.opt_level,
            OptLevel::O1
        );
        assert_eq!(
            parsed(&["--opt-level", "3", "main.sl"]).output.opt_level,
            OptLevel::O3
        );
        let error = parse(&["-O9", "main.sl"]).expect_err("level 9 is not accepted");
        assert_eq!(error.kind(), ErrorKind::ValueValidation);
    }

    #[test]
    fn equals_and_separate_values_are_equivalent() {
        let attached = parsed(&["--target=aarch64-unknown-linux-gnu", "main.sl"]);
        let separate = parsed(&["--target", "aarch64-unknown-linux-gnu", "main.sl"]);
        assert_eq!(attached.target.target, separate.target.target);
        assert_eq!(
            attached.target.target,
            Some(TargetTriple::parse("aarch64-unknown-linux-gnu").expect("test triple is valid"))
        );
    }

    #[test]
    fn remap_specs_accumulate_in_order() {
        let cli = parsed(&[
            "--remap-path-prefix=/a=/b",
            "--remap-path-prefix=/c=/d",
            "main.sl",
        ]);
        assert_eq!(cli.diagnostics.remap_path_prefix, vec!["/a=/b", "/c=/d"]);
    }

    #[test]
    fn double_dash_treats_remaining_arguments_as_inputs() {
        let cli = parsed(&["--", "--not-a-flag.sl"]);
        assert_eq!(cli.inputs, vec![PathBuf::from("--not-a-flag.sl")]);
    }

    #[test]
    fn missing_values_are_reported() {
        let error = parse(&["--target"]).expect_err("a target needs a value");
        assert!(error.to_string().contains("--target"));
    }

    #[test]
    fn unknown_flags_are_rejected() {
        let error = parse(&["--wat", "main.sl"]).expect_err("unknown flags are rejected");
        assert_eq!(error.kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn inputs_are_required_unless_the_action_is_informational() {
        let error = parse(&[]).expect_err("compiling needs an input");
        assert_eq!(error.kind(), ErrorKind::MissingRequiredArgument);
        assert_eq!(
            parse(&["--help"]).expect_err("help stops parsing").kind(),
            ErrorKind::DisplayHelp
        );
        assert_eq!(
            parse(&["--version"])
                .expect_err("version stops parsing")
                .kind(),
            ErrorKind::DisplayVersion
        );
        assert!(parse(&["--explain=E0001"]).is_ok());
        assert!(parse(&["--emit-runtime=seal_rt.a"]).is_ok());
    }

    #[test]
    fn runtime_flags_parse() {
        let exported = parsed(&["--emit-runtime", "seal_rt.a"]);
        assert_eq!(
            exported.runtime.emit_runtime,
            Some(PathBuf::from("seal_rt.a"))
        );
        assert!(exported.inputs.is_empty());

        let linked = parsed(&["--runtime-archive=seal_rt.a", "main.sl"]);
        assert_eq!(
            linked.runtime.runtime_archive,
            Some(PathBuf::from("seal_rt.a"))
        );
        assert_eq!(linked.inputs, vec![PathBuf::from("main.sl")]);
    }

    #[test]
    fn codegen_flags_parse() {
        let cli = parsed(&["-g", "--emit=obj", "-o", "out.o", "main.sl"]);
        assert!(cli.output.debug_info);
        assert_eq!(
            cli.output.requested(),
            RequestedOutput::Artifact(Emit::Object)
        );
        assert_eq!(cli.output.path, Some(PathBuf::from("out.o")));

        assert_eq!(
            parsed(&["main.sl"]).output.requested(),
            RequestedOutput::Artifact(Emit::Executable)
        );
        assert_eq!(
            parsed(&["--emit", "asm", "main.sl"]).output.requested(),
            RequestedOutput::Artifact(Emit::Assembly)
        );
        assert_eq!(
            parsed(&["--emit=check", "main.sl"]).output.requested(),
            RequestedOutput::Check
        );
    }

    #[test]
    fn output_requires_a_single_input() {
        let error =
            parse(&["a.sl", "b.sl", "-o", "out"]).expect_err("one output cannot serve two inputs");
        assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
    }

    #[test]
    fn invalid_emit_kinds_are_rejected() {
        let error = parse(&["--emit=llvm", "main.sl"]).expect_err("unknown artifact kind");
        assert_eq!(error.kind(), ErrorKind::InvalidValue);
    }

    #[test]
    fn max_errors_must_be_positive() {
        let error = parse(&["--max-errors=0", "main.sl"]).expect_err("zero is not a limit");
        assert_eq!(error.kind(), ErrorKind::ValueValidation);
        assert_eq!(
            parsed(&["--max-errors=5", "main.sl"])
                .diagnostics
                .max_errors,
            Some(5)
        );
    }

    #[test]
    fn invalid_targets_and_versions_are_rejected_while_parsing() {
        assert_eq!(
            parse(&["--target=plan9", "main.sl"])
                .expect_err("malformed triple")
                .kind(),
            ErrorKind::ValueValidation
        );
        assert_eq!(
            parse(&["--language-version=next", "main.sl"])
                .expect_err("malformed version")
                .kind(),
            ErrorKind::ValueValidation
        );
    }

    #[test]
    fn help_groups_options_by_concern() {
        let help = Cli::command().render_long_help().to_string();
        for expected in [
            "Optimization:",
            "Output:",
            "Diagnostics:",
            "Target:",
            "Runtime:",
            "--remap-path-prefix",
            "--emit-runtime",
        ] {
            assert!(help.contains(expected), "help is missing `{expected}`");
        }
    }
}
