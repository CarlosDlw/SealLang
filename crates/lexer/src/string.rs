//! String and character literals, including escapes.
//!
//! Cooked values are built while scanning; errors still consume the whole
//! malformed literal so the next token starts at a sane boundary. A literal with
//! any error becomes [`TokenKind::Error`], which the parser treats as a recovery
//! point.

use seal_unicode::is_bidi_control;

use crate::scan::Scanner;
use crate::token::TokenKind;

impl Scanner<'_> {
    pub(crate) fn string(&mut self) -> TokenKind {
        let start = self.cursor.position();
        self.cursor.bump_byte();
        let mut cooked = String::new();
        let mut had_error = false;

        loop {
            let Some(character) = self.cursor.first() else {
                let span = self.span(start, self.cursor.position());
                self.error(codes::E0006, span, "string literal is not terminated")
                    .secondary(
                        self.span(start, start + 1),
                        "this string literal starts here",
                    )
                    .help("add a closing `\"`, or use `\\n` for a line break")
                    .emit();
                had_error = true;
                break;
            };
            match character {
                '"' => {
                    self.cursor.bump_byte();
                    break;
                }
                '\n' | '\r' => {
                    let span = self.span(start, self.cursor.position());
                    self.error(
                        codes::E0006,
                        span,
                        "string literal is not terminated before the line break",
                    )
                    .secondary(
                        self.span(start, start + 1),
                        "this string literal starts here",
                    )
                    .help("add a closing `\"`, or escape the line break with `\\`")
                    .emit();
                    had_error = true;
                    break;
                }
                '\\' => self.escape(&mut cooked, false, &mut had_error),
                character if is_bidi_control(character) => {
                    let span = self.span(
                        self.cursor.position(),
                        self.cursor.position() + character.len_utf8(),
                    );
                    self.report_bidi(character, span);
                    had_error = true;
                    self.cursor.bump_char();
                }
                character if character.is_control() && character != '\t' => {
                    let position = self.cursor.position();
                    let span = self.span(position, position + character.len_utf8());
                    self.error(
                        codes::E0013,
                        span,
                        format!(
                            "raw control character `{}` is not allowed in a string literal",
                            character.escape_debug()
                        ),
                    )
                    .help(format!(
                        "write it as an escape, for example `\\u{{{:X}}}`",
                        character as u32
                    ))
                    .emit();
                    had_error = true;
                    self.cursor.bump_char();
                }
                character => {
                    cooked.push(character);
                    self.cursor.bump_char();
                }
            }
        }

        if had_error {
            TokenKind::Error
        } else {
            TokenKind::Str(self.interner.intern(&cooked))
        }
    }

    pub(crate) fn character(&mut self) -> TokenKind {
        let start = self.cursor.position();
        self.cursor.bump_byte();
        let mut had_error = false;

        match self.cursor.first() {
            None | Some('\n') | Some('\r') => {
                let span = self.span(start, self.cursor.position());
                self.error(codes::E0010, span, "character literal is not terminated")
                    .secondary(self.span(start, start + 1), "this literal starts here")
                    .help("add a closing `'`")
                    .emit();
                return TokenKind::Error;
            }
            Some('\'') => {
                self.cursor.bump_byte();
                let span = self.span(start, self.cursor.position());
                self.error(codes::E0010, span, "character literal is empty")
                    .help("write `' '` for a space, or use `\"\"` for an empty string")
                    .emit();
                return TokenKind::Error;
            }
            _ => {}
        }

        let mut cooked = String::new();
        if self.cursor.first() == Some('\\') {
            self.escape(&mut cooked, true, &mut had_error);
        } else if let Some(character) = self.cursor.bump_char() {
            if is_bidi_control(character) {
                let span = self.span(start, self.cursor.position());
                self.report_bidi(character, span);
                had_error = true;
            } else if character.is_control() && character != '\t' {
                let span = self.span(start, self.cursor.position());
                self.error(
                    codes::E0013,
                    span,
                    format!(
                        "raw control character `{}` is not allowed in a character literal",
                        character.escape_debug()
                    ),
                )
                .help(format!(
                    "write it as an escape, for example `\\u{{{:X}}}`",
                    character as u32
                ))
                .emit();
                had_error = true;
            }
            cooked.push(character);
        }

        if self.cursor.eat_byte(b'\'') {
            // Closed correctly.
        } else if matches!(self.cursor.first(), None | Some('\n') | Some('\r')) {
            let span = self.span(start, self.cursor.position());
            self.error(codes::E0010, span, "character literal is not terminated")
                .secondary(self.span(start, start + 1), "this literal starts here")
                .help("add a closing `'` before the line break")
                .emit();
            had_error = true;
        } else {
            let span = self.span(start, self.cursor.position());
            self.error(
                codes::E0010,
                span,
                "character literal must contain exactly one character",
            )
            .secondary(span, "this literal is not a single character")
            .help("use a string literal for more than one character, and check the closing `'`")
            .emit();
            had_error = true;
            while let Some(character) = self.cursor.first() {
                if character == '\'' {
                    self.cursor.bump_byte();
                    break;
                }
                if character == '\n' || character == '\r' {
                    break;
                }
                self.cursor.bump_char();
            }
        }

        let mut characters = cooked.chars();
        match (characters.next(), characters.next()) {
            (Some(character), None) if !had_error => TokenKind::Char(character),
            _ => TokenKind::Error,
        }
    }

    fn escape(&mut self, cooked: &mut String, in_char: bool, had_error: &mut bool) {
        let start = self.cursor.position();
        self.cursor.bump_byte();
        let Some(character) = self.cursor.first() else {
            let span = self.span(start, self.cursor.position());
            self.error(codes::E0005, span, "escape sequence is incomplete")
                .primary(span, "the file ends after this backslash")
                .emit();
            *had_error = true;
            return;
        };

        match character {
            '0' => {
                self.cursor.bump_byte();
                cooked.push('\0');
            }
            't' => {
                self.cursor.bump_byte();
                cooked.push('\t');
            }
            'n' => {
                self.cursor.bump_byte();
                cooked.push('\n');
            }
            'r' => {
                self.cursor.bump_byte();
                cooked.push('\r');
            }
            '"' => {
                self.cursor.bump_byte();
                cooked.push('"');
            }
            '\'' => {
                self.cursor.bump_byte();
                cooked.push('\'');
            }
            '\\' => {
                self.cursor.bump_byte();
                cooked.push('\\');
            }
            'x' => {
                self.cursor.bump_byte();
                self.hex_escape(start, cooked, had_error);
            }
            'u' => {
                self.cursor.bump_byte();
                self.unicode_escape(start, cooked, had_error);
            }
            '\n' => {
                self.cursor.bump_byte();
                if in_char {
                    self.continuation_in_char(start, had_error);
                } else {
                    self.skip_continuation_whitespace();
                }
            }
            '\r' => {
                self.cursor.bump_byte();
                if self.cursor.eat_byte(b'\n') {
                    if in_char {
                        self.continuation_in_char(start, had_error);
                    } else {
                        self.skip_continuation_whitespace();
                    }
                } else {
                    let span = self.span(start, self.cursor.position());
                    self.error(
                        codes::E0005,
                        span,
                        "line continuation must use a line feed or carriage-return/line-feed pair",
                    )
                    .help("remove the backslash before a bare carriage return")
                    .emit();
                    *had_error = true;
                }
            }
            character => {
                self.cursor.bump_char();
                let span = self.span(start, self.cursor.position());
                self.error(
                    codes::E0005,
                    span,
                    format!("unknown escape sequence `\\{}`", character.escape_debug()),
                )
                .secondary(span, "this escape is not recognized")
                .help("valid escapes are `\\0 \\t \\n \\r \\\" \\' \\\\ \\xNN \\u{...}`")
                .emit();
                *had_error = true;
                cooked.push(character);
            }
        }
    }

    fn continuation_in_char(&mut self, start: usize, had_error: &mut bool) {
        let span = self.span(start, self.cursor.position());
        self.error(
            codes::E0005,
            span,
            "a line continuation is not allowed in a character literal",
        )
        .help("use a string literal if the value spans lines")
        .emit();
        *had_error = true;
    }

    fn skip_continuation_whitespace(&mut self) {
        loop {
            match self.cursor.first_byte() {
                Some(b' ') | Some(b'\t') | Some(b'\n') => {
                    self.cursor.bump_byte();
                }
                Some(b'\r') => {
                    self.cursor.bump_byte();
                    self.cursor.eat_byte(b'\n');
                }
                _ => break,
            }
        }
    }

    fn hex_escape(&mut self, start: usize, cooked: &mut String, had_error: &mut bool) {
        let mut value = 0u32;
        let mut digits = 0;
        while digits < 2 {
            let Some(byte) = self.cursor.first_byte() else {
                break;
            };
            let Some(digit) = (byte as char).to_digit(16) else {
                break;
            };
            value = value * 16 + digit;
            digits += 1;
            self.cursor.bump_byte();
        }
        let span = self.span(start, self.cursor.position());
        if digits != 2 {
            self.error(
                codes::E0005,
                span,
                "`\\x` escape needs exactly two hexadecimal digits",
            )
            .help("write `\\x41`, or use `\\u{...}` for a code point above 0x7F")
            .emit();
            *had_error = true;
            return;
        }
        if value > 0x7F {
            self.error(
                codes::E0005,
                span,
                "`\\x` escape is outside the allowed range 0x00..=0x7F",
            )
            .help("use `\\u{...}` for characters above 0x7F")
            .emit();
            *had_error = true;
            return;
        }
        if let Some(character) = char::from_u32(value) {
            cooked.push(character);
        }
    }

    fn unicode_escape(&mut self, start: usize, cooked: &mut String, had_error: &mut bool) {
        if !self.cursor.eat_byte(b'{') {
            let span = self.span(start, self.cursor.position());
            self.error(codes::E0005, span, "Unicode escapes require braces")
                .help("write `\\u{1F980}`")
                .emit();
            *had_error = true;
            return;
        }

        let mut value = 0u32;
        let mut digits = 0;
        let mut closed = false;
        let mut too_many = false;
        loop {
            match self.cursor.first_byte() {
                Some(b'}') => {
                    self.cursor.bump_byte();
                    closed = true;
                    break;
                }
                Some(byte) if byte.is_ascii_hexdigit() => {
                    if digits == 6 {
                        too_many = true;
                    } else {
                        let digit = (byte as char).to_digit(16).unwrap_or(0);
                        value = value * 16 + digit;
                        digits += 1;
                    }
                    self.cursor.bump_byte();
                }
                _ => break,
            }
        }

        let span = self.span(start, self.cursor.position());
        if digits == 0 {
            self.error(
                codes::E0005,
                span,
                "Unicode escape needs at least one hexadecimal digit",
            )
            .help("write `\\u{41}` for `A`")
            .emit();
            *had_error = true;
            return;
        }
        if too_many {
            self.error(
                codes::E0005,
                span,
                "Unicode escape has more than six hexadecimal digits",
            )
            .help("a code point is at most 0x10FFFF, which needs six digits")
            .emit();
            *had_error = true;
            return;
        }
        if !closed {
            self.error(codes::E0005, span, "Unicode escape is not closed with `}`")
                .help("write `\\u{1F980}`")
                .emit();
            *had_error = true;
            return;
        }
        match char::from_u32(value) {
            Some(character) => cooked.push(character),
            None => {
                self.error(
                    codes::E0005,
                    span,
                    "Unicode escape is not a valid Unicode scalar value",
                )
                .help("surrogates (0xD800..=0xDFFF) and values above 0x10FFFF are not characters")
                .emit();
                *had_error = true;
            }
        }
    }
}

use seal_error_codes as codes;
