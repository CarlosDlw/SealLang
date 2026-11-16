//! Comments: nested block comments and doc comments as trivia.
//!
//! Comments never become significant tokens. They are preserved in source
//! order so documentation tooling and formatters can use them without re-lexing,
//! and block comments participate in newline tracking: a line break hidden
//! inside `/* ... */` still sets `preceded_by_newline` on the next token.

use seal_unicode::is_bidi_control;

use crate::scan::Scanner;
use crate::token::TriviaKind;

impl Scanner<'_> {
    pub(crate) fn line_comment(&mut self) {
        let start = self.cursor.position();
        let kind = match self.cursor.byte_at(2) {
            Some(b'/') if self.cursor.byte_at(3) != Some(b'/') => TriviaKind::OuterLineDoc,
            Some(b'!') => TriviaKind::InnerLineDoc,
            _ => TriviaKind::LineComment,
        };
        self.cursor.bump_byte();
        self.cursor.bump_byte();

        while let Some(character) = self.cursor.first() {
            if character == '\n' || character == '\r' {
                break;
            }
            if is_bidi_control(character) {
                let position = self.cursor.position();
                let span = self.span(position, position + character.len_utf8());
                self.report_bidi(character, span);
            }
            self.cursor.bump_char();
        }
        self.push_trivia(kind, start);
    }

    pub(crate) fn block_comment(&mut self) {
        let start = self.cursor.position();
        let kind = match self.cursor.byte_at(2) {
            Some(b'*') if !matches!(self.cursor.byte_at(3), Some(b'*') | Some(b'/')) => {
                TriviaKind::OuterBlockDoc
            }
            Some(b'!') => TriviaKind::InnerBlockDoc,
            _ => TriviaKind::BlockComment,
        };
        self.cursor.bump_byte();
        self.cursor.bump_byte();

        let mut depth = 1usize;
        loop {
            let Some(character) = self.cursor.first() else {
                let span = self.span(start, self.cursor.position());
                self.error(
                    codes::E0011,
                    span,
                    "unterminated block comment",
                )
                .secondary(self.span(start, start + 2), "this comment starts here")
                .help("add the missing `*/`; block comments nest, so every `/*` needs its own `*/`")
                .emit();
                break;
            };

            if self.cursor.first_byte() == Some(b'/') && self.cursor.second_byte() == Some(b'*') {
                self.cursor.bump_byte();
                self.cursor.bump_byte();
                depth += 1;
                continue;
            }
            if self.cursor.first_byte() == Some(b'*') && self.cursor.second_byte() == Some(b'/') {
                self.cursor.bump_byte();
                self.cursor.bump_byte();
                depth -= 1;
                if depth == 0 {
                    break;
                }
                continue;
            }

            match character {
                '\n' => {
                    self.cursor.bump_byte();
                    self.newline_pending = true;
                }
                '\r' => {
                    self.cursor.bump_byte();
                    if !self.cursor.eat_byte(b'\n') {
                        let position = self.cursor.position() - 1;
                        let span = self.span(position, self.cursor.position());
                        self.error(
                            codes::E0004,
                            span,
                            "bare carriage return is not a line break in Seal",
                        )
                        .primary(span, "this carriage return is not followed by a line feed")
                        .help("use `\\n` or `\\r\\n` between lines")
                        .emit();
                    }
                    self.newline_pending = true;
                }
                character if is_bidi_control(character) => {
                    let position = self.cursor.position();
                    let span = self.span(position, position + character.len_utf8());
                    self.report_bidi(character, span);
                    self.cursor.bump_char();
                }
                _ => {
                    self.cursor.bump_char();
                }
            }
        }

        self.push_trivia(kind, start);
    }
}

use seal_error_codes as codes;
