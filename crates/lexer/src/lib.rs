//! The Seal lexer.
//!
//! Seal source is tokenized in one pass. The lexer is context-free: the same
//! text always produces the same tokens, independent of the parser or the
//! target. It produces [`Lexed`], which keeps significant tokens and comments
//! (trivia) separately, and reports every lexical error through
//! [`seal_diagnostics`] with a stable code in the `E00xx` range.
//!
//! # Guarantees
//!
//! - The token stream always ends with [`TokenKind::Eof`] and tokens never
//!   overlap or cross the file boundary.
//! - A malformed literal becomes [`TokenKind::Error`] covering the whole lexeme,
//!   so the parser can recover at a known boundary; lexing continues afterwards.
//! - The lexer never panics on user input and always makes progress. Internal
//!   invariant violations are reported as an ICE, not a silent loop.
//! - Comments are preserved as trivia, including nested block comments and the
//!   four doc-comment styles; newlines inside block comments set
//!   [`Token::preceded_by_newline`] on the next token.
//! - No global state: interning and diagnostics come from the caller, so two
//!   files can be lexed concurrently.
//!
//! # Literals
//!
//! Numeric bases (`0b`, `0o`, `0x`), separators between digits, decimal
//! exponents, and the numeric type suffixes from the README are supported. A
//! float needs an integer part (`1.`, `1.5`, `1e3`), so a leading dot stays a
//! separate token and `1..2` is not a float. See the `number` module for the
//! exact grammar. Strings support the usual escapes
//! plus `\xNN` (up to 0x7F) and `\u{...}`, and a trailing backslash continues a
//! line. Source text is rejected if it contains bidirectional formatting
//! characters, raw control characters, non-ASCII whitespace, a bare carriage
//! return, or a byte order mark past the start of the file.

#![warn(missing_docs)]

mod comment;
mod cursor;
mod keyword;
mod number;
mod scan;
mod string;
mod token;

#[cfg(test)]
mod tests;

use seal_diagnostics::DiagCtxt;
use seal_span::SourceFile;
use seal_symbol::Interner;

pub use token::{Lexed, NumericBase, NumericSuffix, Token, TokenKind, Trivia, TriviaKind};

/// Lexes `file`, reporting errors through `dcx` and interning names and string
/// contents in `interner`.
///
/// The result is produced even when errors were reported; callers check
/// `dcx.abort_if_errors()` before depending on the tokens semantically.
pub fn lex(file: &SourceFile, dcx: &DiagCtxt, interner: &Interner) -> Lexed {
    scan::Scanner::new(file, dcx, interner).run()
}
