//! The scanner: whitespace, identifiers, punctuation, and the main loop.
//!
//! The scanner is a single pass over the file text. It owns the token and trivia
//! vectors, and it never looks at another file. Every path either consumes at
//! least one byte or emits a diagnostic and consumes one byte; the loop asserts
//! that progress was made, so malformed input cannot hang the compiler.

use seal_diagnostics::{DiagCtxt, ErrorBuilder};
use seal_error_codes as codes;
use seal_error_codes::ErrorCode;
use seal_span::{SourceFile, Span};
use seal_symbol::Interner;
use seal_unicode::{is_bidi_control, is_ident_continue, is_ident_start};

use crate::cursor::Cursor;
use crate::keyword;
use crate::token::{Lexed, Token, TokenKind, Trivia};

/// One lexing pass over a source file.
pub(crate) struct Scanner<'a> {
    pub(crate) cursor: Cursor<'a>,
    pub(crate) file: &'a SourceFile,
    pub(crate) dcx: &'a DiagCtxt,
    pub(crate) interner: &'a Interner,
    pub(crate) tokens: Vec<Token>,
    pub(crate) trivia: Vec<Trivia>,
    pub(crate) newline_pending: bool,
}

impl<'a> Scanner<'a> {
    /// Creates a scanner for `file`.
    pub(crate) fn new(file: &'a SourceFile, dcx: &'a DiagCtxt, interner: &'a Interner) -> Self {
        Self {
            cursor: Cursor::new(file.text()),
            file,
            dcx,
            interner,
            tokens: Vec::new(),
            trivia: Vec::new(),
            newline_pending: false,
        }
    }

    /// Runs the scanner to end of file and returns the result.
    pub(crate) fn run(mut self) -> Lexed {
        while !self.cursor.is_eof() {
            let start = self.cursor.position();
            if let Some(kind) = self.scan_token() {
                self.push_token(kind, start);
            }
            if self.cursor.position() == start {
                self.dcx
                    .span_bug(self.span(start, start), "lexer made no progress");
            }
        }

        let end = self.cursor.position();
        self.tokens.push(Token {
            kind: TokenKind::Eof,
            span: self.span(end, end),
            preceded_by_newline: self.newline_pending,
        });

        Lexed {
            tokens: self.tokens,
            trivia: self.trivia,
        }
    }

    fn scan_token(&mut self) -> Option<TokenKind> {
        let character = self.cursor.first()?;
        match character {
            ' ' | '\t' | '\n' | '\r' => {
                self.whitespace();
                None
            }
            '/' if self.cursor.second_byte() == Some(b'/') => {
                self.line_comment();
                None
            }
            '/' if self.cursor.second_byte() == Some(b'*') => {
                self.block_comment();
                None
            }
            '"' => Some(self.string()),
            '\'' => Some(self.character()),
            '0'..='9' => Some(self.number()),
            _ if is_punctuation(character) => Some(self.punctuation()),
            _ if is_ident_start(character) => Some(self.identifier()),
            _ => Some(self.unknown_character(character)),
        }
    }

    fn push_token(&mut self, kind: TokenKind, start: usize) {
        self.tokens.push(Token {
            kind,
            span: self.span(start, self.cursor.position()),
            preceded_by_newline: self.newline_pending,
        });
        self.newline_pending = false;
    }

    pub(crate) fn push_trivia(&mut self, kind: crate::token::TriviaKind, start: usize) {
        self.trivia.push(Trivia {
            kind,
            span: self.span(start, self.cursor.position()),
        });
    }

    fn whitespace(&mut self) {
        loop {
            match self.cursor.first_byte() {
                Some(b' ') | Some(b'\t') => {
                    self.cursor.bump_byte();
                }
                Some(b'\n') => {
                    self.cursor.bump_byte();
                    self.newline_pending = true;
                }
                Some(b'\r') => {
                    self.cursor.bump_byte();
                    if !self.cursor.eat_byte(b'\n') {
                        let span = self.span(self.cursor.position() - 1, self.cursor.position());
                        self.error(
                            codes::E0004,
                            span,
                            "bare carriage return is not a line break in Seal",
                        )
                        .secondary(span, "this carriage return is not followed by a line feed")
                        .help("use `\\n` or `\\r\\n` between lines")
                        .emit();
                    }
                    self.newline_pending = true;
                }
                _ => break,
            }
        }
    }

    fn identifier(&mut self) -> TokenKind {
        let start = self.cursor.position();
        while let Some(character) = self.cursor.first() {
            if is_ident_continue(character) {
                self.cursor.bump_char();
            } else {
                break;
            }
        }
        let text = self.cursor.slice(start..self.cursor.position());
        match keyword::lookup(text) {
            Some(kind) => kind,
            None => TokenKind::Ident(self.interner.intern(text)),
        }
    }

    fn unknown_character(&mut self, character: char) -> TokenKind {
        let start = self.cursor.position();
        let end = start + character.len_utf8();
        let span = self.span(start, end);

        if is_bidi_control(character) {
            self.report_bidi(character, span);
            self.cursor.bump_char();
            return TokenKind::Error;
        }

        if character == '\u{feff}' {
            self.error(
                codes::E0004,
                span,
                "byte order mark is only allowed at the start of a file",
            )
            .secondary(
                span,
                "this byte order mark appears in the middle of the file",
            )
            .help("remove it; the compiler already strips the BOM at the start of a file")
            .emit();
        } else if character.is_whitespace() {
            self.error(
                codes::E0004,
                span,
                format!(
                    "whitespace character `{}` (U+{:04X}) is not accepted in source text",
                    character.escape_debug(),
                    character as u32
                ),
            )
            .secondary(span, "Seal accepts only ASCII space, tab, and line breaks")
            .help("replace it with a regular space or a line break")
            .emit();
        } else if character.is_control() {
            self.error(
                codes::E0004,
                span,
                format!(
                    "control character `{}` (U+{:04X}) is not allowed outside a literal",
                    character.escape_debug(),
                    character as u32
                ),
            )
            .secondary(span, "this control character has no token")
            .help("remove it, or write it as an escape inside a string literal")
            .emit();
        } else {
            self.error(
                codes::E0004,
                span,
                format!(
                    "character `{}` (U+{:04X}) is not valid Seal source",
                    character.escape_debug(),
                    character as u32
                ),
            )
            .secondary(span, "this character cannot start a token")
            .emit();
        }

        self.cursor.bump_char();
        TokenKind::Error
    }

    fn punctuation(&mut self) -> TokenKind {
        let byte = self
            .cursor
            .bump_byte()
            .unwrap_or_else(|| self.dcx.bug("punctuation called at end of file"));
        match byte {
            b'(' => TokenKind::LParen,
            b')' => TokenKind::RParen,
            b'{' => TokenKind::LBrace,
            b'}' => TokenKind::RBrace,
            b'[' => TokenKind::LBracket,
            b']' => TokenKind::RBracket,
            b',' => TokenKind::Comma,
            b';' => TokenKind::Semicolon,
            b':' => {
                if self.cursor.eat_byte(b':') {
                    TokenKind::ColonColon
                } else {
                    TokenKind::Colon
                }
            }
            b'.' => {
                if self.cursor.eat_byte(b'.') {
                    TokenKind::DotDot
                } else {
                    TokenKind::Dot
                }
            }
            b'~' => TokenKind::Tilde,
            b'+' => {
                if self.cursor.eat_byte(b'+') {
                    TokenKind::PlusPlus
                } else if self.cursor.eat_byte(b'=') {
                    TokenKind::PlusEq
                } else {
                    TokenKind::Plus
                }
            }
            b'-' => {
                if self.cursor.eat_byte(b'-') {
                    TokenKind::MinusMinus
                } else if self.cursor.eat_byte(b'=') {
                    TokenKind::MinusEq
                } else if self.cursor.eat_byte(b'>') {
                    TokenKind::Arrow
                } else {
                    TokenKind::Minus
                }
            }
            b'*' => {
                if self.cursor.eat_byte(b'=') {
                    TokenKind::StarEq
                } else {
                    TokenKind::Star
                }
            }
            b'/' => {
                if self.cursor.eat_byte(b'=') {
                    TokenKind::SlashEq
                } else {
                    TokenKind::Slash
                }
            }
            b'%' => {
                if self.cursor.eat_byte(b'=') {
                    TokenKind::PercentEq
                } else {
                    TokenKind::Percent
                }
            }
            b'=' => {
                if self.cursor.eat_byte(b'=') {
                    TokenKind::EqEq
                } else if self.cursor.eat_byte(b'>') {
                    TokenKind::FatArrow
                } else {
                    TokenKind::Eq
                }
            }
            b'!' => {
                if self.cursor.eat_byte(b'=') {
                    TokenKind::NotEq
                } else {
                    TokenKind::Bang
                }
            }
            b'<' => {
                if self.cursor.eat_byte(b'<') {
                    if self.cursor.eat_byte(b'=') {
                        TokenKind::ShlEq
                    } else {
                        TokenKind::Shl
                    }
                } else if self.cursor.eat_byte(b'=') {
                    TokenKind::LtEq
                } else {
                    TokenKind::Lt
                }
            }
            b'>' => {
                if self.cursor.eat_byte(b'>') {
                    if self.cursor.eat_byte(b'=') {
                        TokenKind::ShrEq
                    } else {
                        TokenKind::Shr
                    }
                } else if self.cursor.eat_byte(b'=') {
                    TokenKind::GtEq
                } else {
                    TokenKind::Gt
                }
            }
            b'&' => {
                if self.cursor.eat_byte(b'&') {
                    TokenKind::AndAnd
                } else if self.cursor.eat_byte(b'=') {
                    TokenKind::AmpEq
                } else {
                    TokenKind::Amp
                }
            }
            b'|' => {
                if self.cursor.eat_byte(b'|') {
                    TokenKind::OrOr
                } else if self.cursor.eat_byte(b'=') {
                    TokenKind::PipeEq
                } else {
                    TokenKind::Pipe
                }
            }
            b'^' => {
                if self.cursor.eat_byte(b'=') {
                    TokenKind::CaretEq
                } else {
                    TokenKind::Caret
                }
            }
            other => {
                let start = self.cursor.position() - 1;
                let span = self.span(start, self.cursor.position());
                self.error(
                    codes::E0004,
                    span,
                    format!("character `{}` is not valid Seal source", other as char),
                )
                .secondary(span, "this character cannot start a token")
                .emit();
                TokenKind::Error
            }
        }
    }

    pub(crate) fn report_bidi(&self, character: char, span: Span) {
        self.error(
            codes::E0012,
            span,
            format!(
                "bidirectional control character U+{:04X} is not allowed in source text",
                character as u32
            ),
        )
        .secondary(
            span,
            "this character can make code display differently from what it does",
        )
        .help(
            "remove it; to produce the character at runtime, write it as a `\\u{...}` escape in a string",
        )
        .emit();
    }

    /// Builds a diagnostic in this file's position space.
    pub(crate) fn error(
        &self,
        code: ErrorCode,
        span: Span,
        message: impl Into<String>,
    ) -> ErrorBuilder<'_> {
        self.dcx
            .error(message)
            .code(code.diag_code())
            .primary(span, "")
    }

    /// Creates a span from file-local byte offsets.
    ///
    /// # Panics
    ///
    /// Panics if the range is outside the file or is not on character
    /// boundaries. Both are lexer bugs: every range comes from cursor positions,
    /// and the cursor only advances over whole characters.
    pub(crate) fn span(&self, start: usize, end: usize) -> Span {
        let start = u32::try_from(start).expect("source positions fit in u32");
        let end = u32::try_from(end).expect("source positions fit in u32");
        self.file
            .span(start..end)
            .expect("lexer ranges are valid for the file")
    }
}

/// Returns `true` for characters that start a punctuation token.
fn is_punctuation(character: char) -> bool {
    matches!(
        character,
        '(' | ')'
            | '{'
            | '}'
            | '['
            | ']'
            | ','
            | ';'
            | ':'
            | '.'
            | '~'
            | '+'
            | '-'
            | '*'
            | '/'
            | '%'
            | '='
            | '!'
            | '<'
            | '>'
            | '&'
            | '|'
            | '^'
    )
}
