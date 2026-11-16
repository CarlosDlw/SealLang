//! Command-line value types and parsers.
//!
//! The compiler's own enums live in the phase crates and cannot implement
//! `clap::ValueEnum` (the orphan rule forbids it). The driver-owned enums here
//! name the accepted spellings and convert into those types, which keeps the
//! command-line vocabulary independent from the compiler's internal enums.

use clap::ValueEnum;
use seal_codegen::Emit;
use seal_diagnostics::ColorChoice;
use seal_session::{ErrorFormat, OptLevel};
use seal_target::TargetTriple;
use seal_version::LanguageVersion;

/// Artifact selected by `--emit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum EmitArg {
    /// Type-check only; write no artifact.
    Check,
    /// Relocatable object file.
    Obj,
    /// Target assembly text.
    Asm,
    /// Linked native executable.
    Exe,
}

/// What a successful compilation should write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RequestedOutput {
    /// No artifact; analysis only.
    Check,
    /// A code generation artifact.
    Artifact(Emit),
}

impl RequestedOutput {
    /// Returns the artifact to generate, or `None` when only checking.
    pub(crate) const fn artifact(self) -> Option<Emit> {
        match self {
            Self::Check => None,
            Self::Artifact(emit) => Some(emit),
        }
    }
}

impl From<EmitArg> for RequestedOutput {
    fn from(emit: EmitArg) -> Self {
        match emit {
            EmitArg::Check => Self::Check,
            EmitArg::Obj => Self::Artifact(Emit::Object),
            EmitArg::Asm => Self::Artifact(Emit::Assembly),
            EmitArg::Exe => Self::Artifact(Emit::Executable),
        }
    }
}

/// Diagnostic format selected by `--error-format`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum ErrorFormatArg {
    /// Human-readable snippets with source context.
    Human,
    /// One JSON object per diagnostic.
    Json,
}

impl From<ErrorFormatArg> for ErrorFormat {
    fn from(format: ErrorFormatArg) -> Self {
        match format {
            ErrorFormatArg::Human => Self::Human,
            ErrorFormatArg::Json => Self::Json,
        }
    }
}

/// Color policy selected by `--color`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum ColorArg {
    /// Color when the destination is a terminal.
    Auto,
    /// Always color.
    Always,
    /// Never color.
    Never,
}

impl From<ColorArg> for ColorChoice {
    fn from(choice: ColorArg) -> Self {
        match choice {
            ColorArg::Auto => Self::Auto,
            ColorArg::Always => Self::Always,
            ColorArg::Never => Self::Never,
        }
    }
}

/// Parses an optimization level for `-O` and `--opt-level`.
pub(crate) fn parse_opt_level(value: &str) -> Result<OptLevel, String> {
    match value {
        "0" => Ok(OptLevel::O0),
        "1" => Ok(OptLevel::O1),
        "2" => Ok(OptLevel::O2),
        "3" => Ok(OptLevel::O3),
        other => Err(format!(
            "expected one of 0, 1, 2, or 3, but found `{other}`"
        )),
    }
}

/// Parses a strictly positive error limit for `--max-errors`.
pub(crate) fn parse_positive(value: &str) -> Result<usize, String> {
    match value.parse::<usize>() {
        Ok(limit) if limit > 0 => Ok(limit),
        _ => Err(format!("expected a positive integer, but found `{value}`")),
    }
}

/// Parses a target triple for `--target`.
pub(crate) fn parse_target(value: &str) -> Result<TargetTriple, String> {
    TargetTriple::parse(value).map_err(|error| error.to_string())
}

/// Parses a language version for `--language-version`.
pub(crate) fn parse_language_version(value: &str) -> Result<LanguageVersion, String> {
    LanguageVersion::parse(value).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use seal_codegen::Emit;
    use seal_diagnostics::ColorChoice;
    use seal_session::{ErrorFormat, OptLevel};

    use super::{
        ColorArg, EmitArg, ErrorFormatArg, RequestedOutput, parse_language_version,
        parse_opt_level, parse_positive, parse_target,
    };

    #[test]
    fn every_optimization_level_has_a_spelling() {
        for (text, level) in [
            ("0", OptLevel::O0),
            ("1", OptLevel::O1),
            ("2", OptLevel::O2),
            ("3", OptLevel::O3),
        ] {
            assert_eq!(parse_opt_level(text), Ok(level));
        }
        assert!(parse_opt_level("4").is_err());
        assert!(parse_opt_level("").is_err());
        assert!(parse_opt_level("O2").is_err());
    }

    #[test]
    fn error_limits_must_be_positive_integers() {
        assert_eq!(parse_positive("1"), Ok(1));
        assert_eq!(parse_positive("250"), Ok(250));
        for rejected in ["0", "-1", "many", "", "1.5"] {
            assert!(parse_positive(rejected).is_err(), "{rejected} was accepted");
        }
    }

    #[test]
    fn targets_and_language_versions_are_validated_eagerly() {
        assert_eq!(
            parse_target("x86_64-unknown-linux-gnu")
                .expect("valid triple")
                .to_string(),
            "x86_64-unknown-linux-gnu"
        );
        assert!(parse_target("mips-unknown-linux").is_err());
        assert_eq!(
            parse_language_version("0.1")
                .expect("valid version")
                .to_string(),
            "0.1"
        );
        assert!(parse_language_version("one").is_err());
    }

    #[test]
    fn emit_kinds_map_to_artifacts_and_check() {
        assert_eq!(
            RequestedOutput::from(EmitArg::Check),
            RequestedOutput::Check
        );
        assert_eq!(RequestedOutput::Check.artifact(), None);
        assert_eq!(
            RequestedOutput::from(EmitArg::Obj),
            RequestedOutput::Artifact(Emit::Object)
        );
        assert_eq!(
            RequestedOutput::from(EmitArg::Asm),
            RequestedOutput::Artifact(Emit::Assembly)
        );
        assert_eq!(
            RequestedOutput::from(EmitArg::Exe).artifact(),
            Some(Emit::Executable)
        );
    }

    #[test]
    fn presentation_choices_convert_to_compiler_types() {
        assert_eq!(ErrorFormat::from(ErrorFormatArg::Json), ErrorFormat::Json);
        assert_eq!(ColorChoice::from(ColorArg::Never), ColorChoice::Never);
    }
}
