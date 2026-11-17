//! Code generation failures.
//!
//! Every variant that a user can trigger carries a documented diagnostic code.
//! [`CodegenError::Internal`] is reserved for broken compiler invariants and is
//! reported as an internal compiler error by the driver.

use std::fmt;
use std::path::PathBuf;

use seal_diagnostics::DiagCode;
use seal_error_codes as codes;

/// Why a program has no usable entry point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryPointError {
    /// No function is named `main`.
    Missing,
    /// More than one function is named `main`.
    Multiple(usize),
    /// `main` takes parameters.
    Parameters(usize),
    /// `main` returns a type other than `i32` or `()`.
    ReturnType(String),
    /// `main` declares generic parameters.
    Generic(usize),
}

impl fmt::Display for EntryPointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => f.write_str(
                "an executable needs a `func main() -> i32` or `func main()` entry point",
            ),
            Self::Multiple(count) => {
                write!(
                    f,
                    "found {count} functions named `main`; exactly one is required"
                )
            }
            Self::Parameters(count) => write!(
                f,
                "the entry point must take no parameters, but `main` declares {count}"
            ),
            Self::ReturnType(found) => write!(
                f,
                "the entry point must return `i32` or `()`, but `main` returns `{found}`"
            ),
            Self::Generic(count) => write!(
                f,
                "the entry point cannot be generic, but `main` declares {count} type parameter(s)"
            ),
        }
    }
}

/// A code generation failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodegenError {
    /// The program has no valid entry point.
    EntryPoint(EntryPointError),
    /// LLVM cannot generate code for the requested target.
    UnsupportedTarget {
        /// The rejected triple.
        triple: String,
        /// The underlying reason.
        detail: String,
    },
    /// Writing or linking the requested artifact failed.
    Artifact {
        /// The output that could not be produced.
        path: PathBuf,
        /// The underlying reason, including linker output when applicable.
        detail: String,
    },
    /// A compiler invariant was violated.
    Internal(String),
}

impl CodegenError {
    /// Creates an internal error.
    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    /// Creates an unsupported-target error.
    pub(crate) fn unsupported_target(triple: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::UnsupportedTarget {
            triple: triple.into(),
            detail: detail.into(),
        }
    }

    /// Creates an artifact error.
    pub(crate) fn artifact(path: impl Into<PathBuf>, detail: impl Into<String>) -> Self {
        Self::Artifact {
            path: path.into(),
            detail: detail.into(),
        }
    }

    /// Returns the diagnostic code for user-triggerable failures.
    ///
    /// Internal errors have no code: the driver reports them through the
    /// internal-compiler-error path instead.
    pub fn diag_code(&self) -> Option<DiagCode> {
        match self {
            Self::EntryPoint(_) => Some(codes::E6001.diag_code()),
            Self::UnsupportedTarget { .. } => Some(codes::E6002.diag_code()),
            Self::Artifact { .. } => Some(codes::E6003.diag_code()),
            Self::Internal(_) => None,
        }
    }
}

impl fmt::Display for CodegenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EntryPoint(error) => error.fmt(f),
            Self::UnsupportedTarget { triple, detail } => {
                write!(f, "cannot generate code for target `{triple}`: {detail}")
            }
            Self::Artifact { path, detail } => {
                write!(f, "could not emit `{}`: {detail}", path.display())
            }
            Self::Internal(message) => write!(f, "internal code generation error: {message}"),
        }
    }
}

impl std::error::Error for CodegenError {}
