//! Fixture helpers for compiler tests.
//!
//! Tests declare what they expect inline, next to the offending source:
//!
//! ```text
//! func main() -> i32 {
//!   let x: i32 = ;
//!   //~^ ERROR E1001: expected expression
//!   x
//! }
//! ```
//!
//! - `//~` annotates the same line.
//! - `//~^` annotates the previous line.
//! - `//~|` annotates the same line as the previous expectation.
//!
//! The runner compares these expectations against the structured diagnostics a
//! test collected, so tests never parse rendered terminal output.

#![warn(missing_docs)]

use std::fmt;

use seal_diagnostics::{DiagCode, Diagnostic, Level};
use seal_span::SourceMap;

/// One expected diagnostic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expectation {
    /// Expected severity.
    pub level: Level,
    /// Expected stable code, if any.
    pub code: Option<String>,
    /// One-based line the diagnostic must point at.
    pub line: u32,
    /// Substring that must appear in the message.
    pub message: String,
}

/// A malformed fixture annotation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpectationError {
    /// One-based line of the annotation.
    pub line: u32,
    /// What is wrong with it.
    pub detail: String,
}

impl fmt::Display for ExpectationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.detail)
    }
}

impl std::error::Error for ExpectationError {}

/// A mismatch between expectations and actual diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mismatch {
    /// One-based line involved, when the mismatch is tied to one.
    pub line: Option<u32>,
    /// Human-readable description.
    pub detail: String,
}

impl fmt::Display for Mismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "line {line}: {}", self.detail),
            None => f.write_str(&self.detail),
        }
    }
}

/// Parses all `//~` annotations in `source`.
pub fn parse_expectations(source: &str) -> Result<Vec<Expectation>, ExpectationError> {
    let mut expectations: Vec<Expectation> = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let line_number = u32::try_from(index + 1).unwrap_or(u32::MAX);
        for segment in line.split("//~").skip(1) {
            let segment = segment.trim();
            let target_line = if let Some(rest) = segment.strip_prefix('^') {
                let previous = line_number.checked_sub(1).ok_or_else(|| ExpectationError {
                    line: line_number,
                    detail: "`^` annotation has no previous line".to_owned(),
                })?;
                (previous, rest)
            } else if let Some(rest) = segment.strip_prefix('|') {
                let previous = expectations
                    .last()
                    .map(|expectation| expectation.line)
                    .ok_or_else(|| ExpectationError {
                        line: line_number,
                        detail: "`|` annotation has no previous expectation".to_owned(),
                    })?;
                (previous, rest)
            } else {
                (line_number, segment)
            };
            let (target_line, rest) = target_line;
            let rest = rest.trim_start();

            let (level_word, rest) = split_token(rest);
            let level = match level_word {
                "ERROR" => Level::Error,
                "WARNING" | "WARN" => Level::Warning,
                "NOTE" => Level::Note,
                "HELP" => Level::Help,
                other => {
                    return Err(ExpectationError {
                        line: line_number,
                        detail: format!(
                            "unknown annotation level `{other}`; expected ERROR, WARNING, NOTE, or HELP"
                        ),
                    });
                }
            };

            let (first, after_first) = split_token(rest);
            let (code, text) = if is_diagnostic_code(first) {
                (Some(first.to_owned()), after_first)
            } else {
                (None, rest)
            };
            let message = text.trim_start_matches(':').trim().to_owned();

            expectations.push(Expectation {
                level,
                code,
                line: target_line,
                message,
            });
        }
    }
    Ok(expectations)
}

/// Splits the next token, treating `:` as a separator as well as whitespace so
/// that `E1001: message` and `ERROR: message` tokenize as expected.
fn split_token(text: &str) -> (&str, &str) {
    let text = text.trim_start();
    match text.find(|character: char| character.is_whitespace() || character == ':') {
        Some(position) => (&text[..position], &text[position..]),
        None => (text, ""),
    }
}

fn is_diagnostic_code(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 5
        && matches!(bytes[0], b'E' | b'W' | b'I')
        && bytes[1..].iter().all(u8::is_ascii_digit)
}

/// Compares expectations with collected diagnostics.
///
/// Matching is by severity, code, start line, and message substring. Every
/// expectation must match a distinct diagnostic, and every diagnostic must be
/// expected.
pub fn check(
    source_map: &SourceMap,
    expectations: &[Expectation],
    diagnostics: &[Diagnostic],
) -> Result<(), Vec<Mismatch>> {
    let mut used = vec![false; diagnostics.len()];
    let mut mismatches = Vec::new();

    for expectation in expectations {
        let found = diagnostics
            .iter()
            .enumerate()
            .find_map(|(index, diagnostic)| {
                if used[index] {
                    return None;
                }
                let line = diagnostic
                    .labels()
                    .first()
                    .and_then(|label| source_map.line_col(label.span))
                    .map(|(_, start, _)| start.line);
                let code_matches = match (&expectation.code, diagnostic.code()) {
                    (None, None) => true,
                    (Some(expected), Some(actual)) => expected == actual.as_str(),
                    _ => false,
                };
                let matches = diagnostic.level() == expectation.level
                    && code_matches
                    && line == Some(expectation.line)
                    && diagnostic.message().contains(&expectation.message);
                matches.then_some(index)
            });

        match found {
            Some(index) => used[index] = true,
            None => mismatches.push(Mismatch {
                line: Some(expectation.line),
                detail: format!(
                    "expected {} {} on this line, found nothing matching {:?}",
                    expectation.level,
                    expectation.code.as_deref().unwrap_or(""),
                    expectation.message
                )
                .trim()
                .to_owned(),
            }),
        }
    }

    for (index, diagnostic) in diagnostics.iter().enumerate() {
        if !used[index] {
            mismatches.push(Mismatch {
                line: diagnostic
                    .labels()
                    .first()
                    .and_then(|label| source_map.line_col(label.span))
                    .map(|(_, start, _)| start.line),
                detail: format!(
                    "unexpected {} {}: {}",
                    diagnostic.level(),
                    diagnostic.code().map_or("", DiagCode::as_str),
                    diagnostic.message()
                )
                .trim()
                .to_owned(),
            });
        }
    }

    if mismatches.is_empty() {
        Ok(())
    } else {
        Err(mismatches)
    }
}

#[cfg(test)]
mod tests {
    use seal_diagnostics::{Diagnostic, LabelStyle, Level};
    use seal_span::{FileName, SourceMap};

    use super::{check, parse_expectations};

    #[test]
    fn parses_same_line_annotations() {
        let source = "let x = ;\n//~ ERROR E1001: expected expression\n";
        let expectations = parse_expectations(source).expect("annotations parse");
        assert_eq!(expectations.len(), 1);
        assert_eq!(expectations[0].line, 2);
        assert_eq!(expectations[0].level, Level::Error);
        assert_eq!(expectations[0].code.as_deref(), Some("E1001"));
        assert_eq!(expectations[0].message, "expected expression");
    }

    #[test]
    fn caret_and_pipe_annotations() {
        let source = "let x = ;\n//~^ ERROR: missing value\n//~| NOTE: second\n";
        let expectations = parse_expectations(source).expect("annotations parse");
        assert_eq!(expectations.len(), 2);
        assert_eq!(expectations[0].line, 1);
        assert_eq!(expectations[0].code, None);
        assert_eq!(expectations[0].message, "missing value");
        assert_eq!(expectations[1].line, 1);
        assert_eq!(expectations[1].level, Level::Note);
    }

    #[test]
    fn warnings_and_optional_codes() {
        let source = "x\n//~ WARNING unused_variable\n";
        let expectations = parse_expectations(source).expect("annotations parse");
        assert_eq!(expectations[0].level, Level::Warning);
        assert_eq!(expectations[0].code, None);
        assert_eq!(expectations[0].message, "unused_variable");
    }

    #[test]
    fn malformed_annotations_are_rejected() {
        assert!(parse_expectations("//~ BOOM\n").is_err());
        assert!(parse_expectations("//~^\n").is_err());
        assert!(parse_expectations("//~|\n").is_err());
    }

    #[test]
    fn check_matches_structured_diagnostics() {
        let map = SourceMap::new();
        let id = map
            .add_file(FileName::virtual_name("t"), "let x = ;\n".to_owned())
            .expect("file registers");
        let span = map.span(id, 8..9).expect("valid span");
        let diagnostic = Diagnostic::new(Level::Error, "expected expression")
            .with_code(seal_diagnostics::DiagCode::new("E1001"))
            .with_label(span, "", LabelStyle::Primary);
        let expectations = parse_expectations("let x = ;\n//~^ ERROR E1001: expected expression\n")
            .expect("annotations parse");
        assert!(check(&map, &expectations, std::slice::from_ref(&diagnostic)).is_ok());
    }

    #[test]
    fn check_reports_missing_and_unexpected() {
        let map = SourceMap::new();
        let id = map
            .add_file(FileName::virtual_name("t"), "abc\n".to_owned())
            .expect("file registers");
        let span = map.span(id, 0..1).expect("valid span");
        let diagnostic =
            Diagnostic::new(Level::Warning, "unused").with_label(span, "", LabelStyle::Primary);
        let expectations = parse_expectations("abc\n//~ ERROR: missing\n").expect("valid");
        let mismatches =
            check(&map, &expectations, std::slice::from_ref(&diagnostic)).expect_err("mismatch");
        assert_eq!(mismatches.len(), 2);
    }
}
