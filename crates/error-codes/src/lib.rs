//! The stable diagnostic-code registry.
//!
//! Codes are a public interface: tooling filters on them, and users search for
//! them. Once assigned, a code never changes meaning and is never reused, even
//! if the diagnostic is retired.
//!
//! Ranges are allocated per compiler phase:
//!
//! | Range | Phase |
//! |-------|-------|
//! | `E0xxx` | source loading / lexer |
//! | `E1xxx` | parser |
//! | `E2xxx` | name resolution |
//! | `E3xxx` | type checking |
//! | `E4xxx` | const evaluation / patterns |
//! | `E5xxx` | MIR / ownership |
//! | `E6xxx` | code generation / linking |
//! | `E9xxx` | compiler internals |
//!
//! Warnings are identified by lint name, not by code.

#![warn(missing_docs)]

use seal_diagnostics::DiagCode;

/// A documented diagnostic code.
#[derive(Clone, Copy, Debug)]
pub struct ErrorCode {
    code: &'static str,
    title: &'static str,
    explanation: &'static str,
}

impl ErrorCode {
    /// Returns the stable code text.
    #[inline]
    pub const fn code(self) -> &'static str {
        self.code
    }

    /// Returns the one-line title.
    #[inline]
    pub const fn title(self) -> &'static str {
        self.title
    }

    /// Returns the long-form explanation.
    #[inline]
    pub const fn explanation(self) -> &'static str {
        self.explanation
    }

    /// Returns the code as a diagnostic code.
    ///
    /// The registry is the source of truth for code format; a malformed entry is
    /// caught by the registry's own tests before it can reach this method.
    #[inline]
    pub fn diag_code(self) -> DiagCode {
        DiagCode::new(self.code)
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code)
    }
}

/// The source file is not valid UTF-8.
pub const E0001: ErrorCode = ErrorCode {
    code: "E0001",
    title: "source file is not valid UTF-8",
    explanation: "\
Seal source files must be encoded as UTF-8. Unlike some languages, Seal does not
guess an encoding from the locale: a file with invalid UTF-8 is rejected, because
byte offsets in diagnostics and debug information would otherwise be ambiguous.

This error reports the first byte offset that is not part of a valid UTF-8
sequence. Convert the file to UTF-8 (most editors do this by default) and rebuild.",
};

/// The source file exceeds the compiler's size limit.
pub const E0002: ErrorCode = ErrorCode {
    code: "E0002",
    title: "source file is too large",
    explanation: "\
Source positions are 32-bit, so a single file cannot exceed the position space.
The limit is far above any source file a human writes; exceeding it usually
indicates that the wrong file was passed to the compiler (for example, a data or
binary file).

Split the input or verify the path passed to `sealc`.",
};

/// The source file could not be read.
pub const E0003: ErrorCode = ErrorCode {
    code: "E0003",
    title: "could not read source file",
    explanation: "\
The compiler could not read the path given on the command line. The attached note
contains the operating-system error: the file may not exist, may be a directory,
or may lack read permission.

Check the path and the file permissions, then rebuild.",
};

/// The internal compiler error code.
pub const E9001: ErrorCode = ErrorCode {
    code: "E9001",
    title: "internal compiler error",
    explanation: "\
The compiler hit an internal invariant that should never fail. This is a bug in
Seal, not in your program. The report includes the message and, when enabled with
`SEAL_BACKTRACE=1`, a backtrace.

Please report the program that triggered the error together with the compiler
version and the backtrace.",
};

/// Too many errors were emitted.
pub const E9002: ErrorCode = ErrorCode {
    code: "E9002",
    title: "too many errors emitted",
    explanation: "\
The compiler stops after a fixed number of errors to keep the output usable. Later
errors are often consequences of the first ones, so fixing the reported errors
usually makes the rest disappear.

Fix the errors above and rebuild. The limit can be raised with `--max-errors`.",
};

/// Every registered code, in allocation order.
pub static ALL: &[ErrorCode] = &[E0001, E0002, E0003, E9001, E9002];

/// Looks up a code by its text, for example `E0001`.
pub fn lookup(code: &str) -> Option<&'static ErrorCode> {
    ALL.iter().find(|entry| entry.code == code)
}

/// Renders the `--explain` output for a code.
pub fn explain(code: &str) -> Option<String> {
    let entry = lookup(code)?;
    Some(format!(
        "{code}: {}\n\n{}\n",
        entry.title(),
        entry.explanation()
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{ALL, E0001, E0002, E0003, E9001, E9002, explain, lookup};

    #[test]
    fn codes_are_unique_and_well_formed() {
        let mut seen = BTreeSet::new();
        for entry in ALL {
            let code = entry.code();
            assert!(seen.insert(code), "duplicate code {code}");
            assert_eq!(code.len(), 5, "code {code} must have five characters");
            assert!(
                code.starts_with('E'),
                "error codes start with E, got {code}"
            );
            assert!(
                code[1..]
                    .chars()
                    .all(|character| character.is_ascii_digit()),
                "code {code} must end with four digits"
            );
            assert!(!entry.title().is_empty(), "code {code} has no title");
            assert!(
                entry.explanation().len() > 40,
                "code {code} needs a real explanation"
            );
            // The registry's format contract must match the diagnostics crate.
            assert_eq!(entry.diag_code().as_str(), code);
        }
    }

    #[test]
    fn lookup_finds_registered_codes() {
        assert_eq!(
            lookup("E0001").map(|entry| entry.title()),
            Some(E0001.title())
        );
        assert_eq!(
            lookup("E9002").map(|entry| entry.title()),
            Some(E9002.title())
        );
        assert!(lookup("E0000").is_none());
        assert!(lookup("e0001").is_none());
    }

    #[test]
    fn explain_renders_title_and_body() {
        let output = explain("E0003").expect("code exists");
        assert!(output.starts_with("E0003: could not read source file"));
        assert!(output.contains("permission"));
        assert!(explain("E7777").is_none());
    }

    #[test]
    fn codes_are_grouped_by_phase() {
        assert_eq!(E0001.code()[1..2].parse::<u8>().expect("digit"), 0);
        assert_eq!(E0002.code()[1..2].parse::<u8>().expect("digit"), 0);
        assert_eq!(E0003.code()[1..2].parse::<u8>().expect("digit"), 0);
        assert_eq!(E9001.code()[1..2].parse::<u8>().expect("digit"), 9);
        assert_eq!(E9002.code()[1..2].parse::<u8>().expect("digit"), 9);
    }
}
