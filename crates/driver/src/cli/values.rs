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
use seal_version::{Edition, LanguageVersion};

/// Artifact selected by `build --emit`.
///
/// Checking is a separate subcommand rather than an artifact kind, so a build
/// request always names something to write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum EmitArg {
    /// Relocatable object file.
    Obj,
    /// Target assembly text.
    Asm,
    /// Linked native executable.
    Exe,
    /// A `.sealmono` archive of per-CGU LLVM bitcode.
    Mono,
    /// A static C library.
    Staticlib,
    /// A shared C library.
    Cdylib,
    /// A JSON snapshot of the package API and module tree.
    Api,
}

impl From<EmitArg> for Emit {
    fn from(emit: EmitArg) -> Self {
        match emit {
            EmitArg::Obj => Self::Object,
            EmitArg::Asm => Self::Assembly,
            EmitArg::Exe => Self::Executable,
            EmitArg::Mono => Self::Mono,
            EmitArg::Staticlib => Self::StaticLib,
            EmitArg::Cdylib => Self::Cdylib,
            EmitArg::Api => Self::Object,
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

/// Parses a four-digit edition for `--edition`.
pub(crate) fn parse_edition(value: &str) -> Result<Edition, String> {
    Edition::parse(value).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use seal_codegen::Emit;
    use seal_diagnostics::ColorChoice;
    use seal_session::{ErrorFormat, OptLevel};

    use super::{
        ColorArg, EmitArg, ErrorFormatArg, parse_edition, parse_language_version, parse_opt_level,
        parse_positive, parse_target,
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
    fn editions_are_validated_eagerly() {
        assert_eq!(
            parse_edition("2026").expect("valid edition").to_string(),
            "2026"
        );
        assert!(parse_edition("2027").is_err());
        assert!(parse_edition("26").is_err());
    }

    #[test]
    fn emit_kinds_map_to_artifacts() {
        assert_eq!(Emit::from(EmitArg::Obj), Emit::Object);
        assert_eq!(Emit::from(EmitArg::Asm), Emit::Assembly);
        assert_eq!(Emit::from(EmitArg::Exe), Emit::Executable);
        assert_eq!(Emit::from(EmitArg::Mono), Emit::Mono);
        assert_eq!(Emit::from(EmitArg::Staticlib), Emit::StaticLib);
        assert_eq!(Emit::from(EmitArg::Cdylib), Emit::Cdylib);
        assert_eq!(Emit::from(EmitArg::Api), Emit::Object);
    }

    #[test]
    fn presentation_choices_convert_to_compiler_types() {
        assert_eq!(ErrorFormat::from(ErrorFormatArg::Json), ErrorFormat::Json);
        assert_eq!(ColorChoice::from(ColorArg::Never), ColorChoice::Never);
    }
}
