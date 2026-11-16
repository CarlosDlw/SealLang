//! Command-line parsing for `sealc`.
//!
//! The parser is hand-written: compiler flags need exact control over `--`,
//! attached values (`-O2`, `--target=x`), and diagnostics that point at the
//! offending argument. `clap` would add a dependency and still need custom
//! handling for these cases.

use std::fmt;
use std::path::PathBuf;

use seal_diagnostics::ColorChoice;
use seal_session::{ErrorFormat, OptLevel};

/// Parsed command-line arguments.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Args {
    /// Input files.
    pub inputs: Vec<PathBuf>,
    /// Target triple override.
    pub target: Option<String>,
    /// Optimization level (`-O`, `-O0..3`, `--opt-level`).
    pub opt_level: OptLevel,
    /// Diagnostic output format.
    pub error_format: ErrorFormat,
    /// Color policy.
    pub color: ColorChoice,
    /// Language version override.
    pub language_version: Option<String>,
    /// Allow unstable language features.
    pub allow_unstable: bool,
    /// `--remap-path-prefix` specifications.
    pub remap: Vec<String>,
    /// Error limit override.
    pub max_errors: Option<usize>,
    /// Print phase timings.
    pub time_passes: bool,
    /// Print an error-code explanation.
    pub explain: Option<String>,
    /// Print usage.
    pub show_help: bool,
    /// Print version.
    pub show_version: bool,
}

/// A command-line error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ArgError {
    /// A flag that takes a value was the last argument.
    MissingValue(&'static str),
    /// A value was not valid.
    InvalidValue {
        /// The flag.
        flag: &'static str,
        /// The rejected value.
        value: String,
    },
    /// The flag is not recognized.
    UnknownFlag(String),
    /// No input file was given.
    NoInputs,
}

impl fmt::Display for ArgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingValue(flag) => write!(f, "the flag `{flag}` needs a value"),
            Self::InvalidValue { flag, value } => {
                write!(f, "invalid value `{value}` for `{flag}`")
            }
            Self::UnknownFlag(flag) => write!(f, "unknown flag `{flag}`"),
            Self::NoInputs => f.write_str("no input files; run `sealc --help` for usage"),
        }
    }
}

impl std::error::Error for ArgError {}

/// Parses arguments, excluding the program name.
pub(crate) fn parse(argv: impl IntoIterator<Item = String>) -> Result<Args, ArgError> {
    let mut args = Args::default();
    let mut iterator = argv.into_iter().peekable();
    let mut only_inputs = false;

    while let Some(argument) = iterator.next() {
        if only_inputs {
            args.inputs.push(PathBuf::from(argument));
            continue;
        }
        if argument == "--" {
            only_inputs = true;
            continue;
        }

        let (flag, inline) = match argument.split_once('=') {
            Some((flag, value)) => (flag.to_owned(), Some(value.to_owned())),
            None => (argument.clone(), None),
        };

        // `-O`, `-O0`, `-O1`, `-O2`, `-O3`; attached values are not separated by
        // `=`, so they are handled before the general match.
        if let Some(rest) = flag.strip_prefix("-O") {
            if inline.is_some() {
                return Err(ArgError::InvalidValue {
                    flag: "-O",
                    value: argument,
                });
            }
            let level = if rest.is_empty() { "3" } else { rest };
            args.opt_level = parse_opt_level(level)?;
            continue;
        }

        // `-h`/`-V` are accepted alongside the long forms.
        match flag.as_str() {
            "-h" | "--help" => args.show_help = true,
            "-V" | "--version" => args.show_version = true,
            "--allow-unstable-features" => args.allow_unstable = true,
            "--time-passes" => args.time_passes = true,
            "--target" => args.target = Some(take_value(&mut iterator, inline, "--target")?),
            "--opt-level" => {
                let value = take_value(&mut iterator, inline, "--opt-level")?;
                args.opt_level = parse_opt_level(&value)?;
            }
            "--error-format" => {
                let value = take_value(&mut iterator, inline, "--error-format")?;
                args.error_format = match value.as_str() {
                    "human" => ErrorFormat::Human,
                    "json" => ErrorFormat::Json,
                    other => {
                        return Err(ArgError::InvalidValue {
                            flag: "--error-format",
                            value: other.to_owned(),
                        });
                    }
                };
            }
            "--color" => {
                let value = take_value(&mut iterator, inline, "--color")?;
                args.color = match value.as_str() {
                    "auto" => ColorChoice::Auto,
                    "always" => ColorChoice::Always,
                    "never" => ColorChoice::Never,
                    other => {
                        return Err(ArgError::InvalidValue {
                            flag: "--color",
                            value: other.to_owned(),
                        });
                    }
                };
            }
            "--language-version" => {
                args.language_version =
                    Some(take_value(&mut iterator, inline, "--language-version")?);
            }
            "--remap-path-prefix" => {
                args.remap
                    .push(take_value(&mut iterator, inline, "--remap-path-prefix")?);
            }
            "--max-errors" => {
                let value = take_value(&mut iterator, inline, "--max-errors")?;
                match value.parse::<usize>() {
                    Ok(limit) if limit > 0 => args.max_errors = Some(limit),
                    _ => {
                        return Err(ArgError::InvalidValue {
                            flag: "--max-errors",
                            value,
                        });
                    }
                }
            }
            "--explain" => {
                args.explain = Some(take_value(&mut iterator, inline, "--explain")?);
            }
            _ if flag.starts_with('-') => {
                return Err(ArgError::UnknownFlag(flag));
            }
            _ => args.inputs.push(PathBuf::from(argument)),
        }
    }

    let informational = args.show_help || args.show_version || args.explain.is_some();
    if !informational && args.inputs.is_empty() {
        return Err(ArgError::NoInputs);
    }
    Ok(args)
}

fn take_value(
    iterator: &mut impl Iterator<Item = String>,
    inline: Option<String>,
    flag: &'static str,
) -> Result<String, ArgError> {
    match inline {
        Some(value) => Ok(value),
        None => iterator.next().ok_or(ArgError::MissingValue(flag)),
    }
}

fn parse_opt_level(value: &str) -> Result<OptLevel, ArgError> {
    match value {
        "0" => Ok(OptLevel::O0),
        "1" => Ok(OptLevel::O1),
        "2" => Ok(OptLevel::O2),
        "3" => Ok(OptLevel::O3),
        other => Err(ArgError::InvalidValue {
            flag: "--opt-level",
            value: other.to_owned(),
        }),
    }
}

/// Returns the usage text.
pub(crate) fn usage() -> &'static str {
    "\
sealc — the Seal compiler

USAGE:
    scalc [OPTIONS] <INPUT>...

OPTIONS:
    -O[0-3], --opt-level <0-3>   Optimization level (default: 0)
    --target <TRIPLE>            Target triple (default: host)
    --language-version <X.Y>     Language version (default: current)
    --allow-unstable-features    Accept unstable language features
    --error-format <human|json>  Diagnostic format (default: human)
    --color <auto|always|never>  Color policy (default: auto)
    --remap-path-prefix <FROM=TO>
                                 Rewrite paths in diagnostics
    --max-errors <N>             Stop after N errors
    --time-passes                Print phase timings
    --explain <CODE>             Explain a diagnostic code
    -h, --help                   Print this help
    -V, --version                Print the compiler version
    --                           Treat all remaining arguments as inputs
"
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use seal_session::OptLevel;

    use super::{ArgError, Args, parse};

    fn parse_ok(arguments: &[&str]) -> Args {
        parse(arguments.iter().map(|argument| (*argument).to_owned())).expect("arguments parse")
    }

    #[test]
    fn parses_a_single_input() {
        let args = parse_ok(&["main.sl"]);
        assert_eq!(args.inputs, vec![PathBuf::from("main.sl")]);
        assert_eq!(args.opt_level, OptLevel::O0);
        assert!(!args.allow_unstable);
    }

    #[test]
    fn attached_opt_levels() {
        assert_eq!(parse_ok(&["-O", "main.sl"]).opt_level, OptLevel::O3);
        assert_eq!(parse_ok(&["-O2", "main.sl"]).opt_level, OptLevel::O2);
        assert_eq!(
            parse_ok(&["--opt-level=1", "main.sl"]).opt_level,
            OptLevel::O1
        );
        assert_eq!(
            parse_ok(&["--opt-level", "3", "main.sl"]).opt_level,
            OptLevel::O3
        );
        assert!(matches!(
            parse(vec!["-O9".to_owned(), "main.sl".to_owned()]),
            Err(ArgError::InvalidValue { .. })
        ));
    }

    #[test]
    fn equals_and_separate_values_are_equivalent() {
        let attached = parse_ok(&["--target=aarch64-unknown-linux-gnu", "main.sl"]);
        let separate = parse_ok(&["--target", "aarch64-unknown-linux-gnu", "main.sl"]);
        assert_eq!(attached.target, separate.target);
        assert_eq!(
            attached.target.as_deref(),
            Some("aarch64-unknown-linux-gnu")
        );
    }

    #[test]
    fn remap_specs_accumulate() {
        let args = parse_ok(&[
            "--remap-path-prefix=/a=/b",
            "--remap-path-prefix=/c=/d",
            "main.sl",
        ]);
        assert_eq!(args.remap, vec!["/a=/b", "/c=/d"]);
    }

    #[test]
    fn double_dash_treats_remaining_as_inputs() {
        let args = parse_ok(&["--", "--not-a-flag.sl"]);
        assert_eq!(args.inputs, vec![PathBuf::from("--not-a-flag.sl")]);
    }

    #[test]
    fn missing_values_are_rejected() {
        assert_eq!(
            parse(vec!["--target".to_owned()]),
            Err(ArgError::MissingValue("--target"))
        );
    }

    #[test]
    fn unknown_flags_are_rejected() {
        assert_eq!(
            parse(vec!["--wat".to_owned(), "main.sl".to_owned()]),
            Err(ArgError::UnknownFlag("--wat".to_owned()))
        );
    }

    #[test]
    fn inputs_are_required_unless_informational() {
        assert_eq!(parse(Vec::<String>::new()), Err(ArgError::NoInputs));
        assert!(parse(vec!["--help".to_owned()]).is_ok());
        assert!(parse(vec!["--version".to_owned()]).is_ok());
        assert!(parse(vec!["--explain=E0001".to_owned()]).is_ok());
    }

    #[test]
    fn max_errors_must_be_positive() {
        assert_eq!(
            parse(vec!["--max-errors=0".to_owned(), "main.sl".to_owned()]),
            Err(ArgError::InvalidValue {
                flag: "--max-errors",
                value: "0".to_owned()
            })
        );
        assert_eq!(parse_ok(&["--max-errors=5", "main.sl"]).max_errors, Some(5));
    }
}
