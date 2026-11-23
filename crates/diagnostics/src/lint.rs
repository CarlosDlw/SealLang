//! Lint names and levels.
//!
//! A lint is a stable, named policy attached to a warning. The name is part of
//! the command-line and JSON interface; it is not a diagnostic code. The
//! compiler owns the list so the driver and `seal lint` validate against one
//! source of truth.

/// Lint that reports uses of deprecated declarations.
pub const DEPRECATED: &str = "deprecated";

/// Lint that reports an import that binds nothing used by its file.
pub const UNUSED_IMPORT: &str = "unused_import";

/// Every individual lint the compiler can emit.
pub const KNOWN_LINTS: [&str; 2] = [DEPRECATED, UNUSED_IMPORT];

/// Command-level group that selects every warning.
///
/// The group is not a lint in the diagnostic context: `--deny warnings` keeps
/// its existing meaning and denies every warning the compiler emits.
pub const ALL_WARNINGS: &str = "warnings";

/// How a lint is treated when it fires.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub enum LintLevel {
    /// Drop the diagnostic.
    Allow,
    /// Emit a warning.
    #[default]
    Warn,
    /// Emit an error.
    Deny,
}

/// Maps a command-line lint name to its stable static spelling.
#[inline]
pub fn known_lint(name: &str) -> Option<&'static str> {
    KNOWN_LINTS.iter().copied().find(|lint| *lint == name)
}

#[cfg(test)]
mod tests {
    use super::{KNOWN_LINTS, LintLevel, known_lint};

    #[test]
    fn only_registered_lints_resolve() {
        assert_eq!(known_lint("deprecated"), Some("deprecated"));
        assert_eq!(known_lint("unused_import"), Some("unused_import"));
        assert_eq!(known_lint("warnings"), None);
        assert_eq!(known_lint("missing"), None);
    }

    #[test]
    fn known_lints_are_unique() {
        for (index, lint) in KNOWN_LINTS.iter().enumerate() {
            assert!(!KNOWN_LINTS[..index].contains(lint));
        }
    }

    #[test]
    fn the_default_level_is_warn() {
        assert_eq!(LintLevel::default(), LintLevel::Warn);
    }
}
