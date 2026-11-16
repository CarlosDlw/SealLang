//! Diagnostic severity.

use std::fmt;

/// Severity of a diagnostic, ordered from most to least severe.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Level {
    /// An internal compiler error. Emitting one aborts the compiler.
    Bug,
    /// A fatal condition that prevents compilation from starting.
    Fatal,
    /// The program is invalid.
    Error,
    /// The program is valid but suspicious.
    Warning,
    /// Extra context attached to another diagnostic.
    Note,
    /// A suggested action attached to another diagnostic.
    Help,
}

impl Level {
    /// Returns `true` for [`Level::Bug`], [`Level::Fatal`], and [`Level::Error`].
    #[inline]
    pub const fn is_error(self) -> bool {
        matches!(self, Self::Bug | Self::Fatal | Self::Error)
    }

    /// Returns the lowercase name used in human-readable output.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bug => "internal compiler error",
            Self::Fatal => "fatal error",
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Note => "note",
            Self::Help => "help",
        }
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
