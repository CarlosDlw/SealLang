//! Numeric literal scanning.
//!
//! Grammar, in one place:
//!
//! - Decimal integers: digits with separators between digits only.
//! - Prefixed integers: `0b`, `0o`, `0x` (lowercase), then digits of that base.
//! - Decimal floats: digits, an optional `.` followed by digits, an optional
//!   `e`/`E` exponent with optional sign and at least one digit.
//! - A float needs an integer part. `1.` is a float unless the dot starts a
//!   field access, method call, range, or separator (`1.foo`, `1..2`, `1._0`),
//!   so `.5` is a dot followed by an integer.
//! - Suffixes: the numeric types listed in the README. An integer literal with
//!   an `f32`/`f64` suffix is a float.
//!
//! Separators follow Java's rule: they may appear between digits, anywhere,
//! including several in a row, but never at the start, at the end, next to the
//! decimal point, next to the exponent marker, or next to the suffix. This is
//! stricter than Rust and gives the user a diagnostic with a precise span
//! instead of silently accepting `1_`.

use seal_unicode::is_ident_start;

use crate::scan::Scanner;
use crate::token::{NumericBase, NumericSuffix, TokenKind};

impl Scanner<'_> {
    pub(crate) fn number(&mut self) -> TokenKind {
        let start = self.cursor.position();
        let mut had_error = false;
        let mut cleaned = String::new();
        let mut base = NumericBase::Decimal;
        let mut is_float = false;

        if let Some(prefixed) = self.base_prefix(start, &mut had_error) {
            base = prefixed;
            cleaned.clear();
            let has_digits = self.scan_radix_digits(base, &mut cleaned, &mut had_error);
            if !has_digits {
                let span = self.span(start, self.cursor.position());
                self.report_once(
                    &mut had_error,
                    codes::E0007,
                    span,
                    "numeric literal has no digits after its base prefix",
                    "write at least one digit of the chosen base",
                );
            }
            if self.cursor.first_byte() == Some(b'.') {
                let dot = self.cursor.position();
                let span = self.span(dot, dot + 1);
                self.report_once(
                    &mut had_error,
                    codes::E0007,
                    span,
                    "a prefixed numeric literal cannot have a fractional part",
                    "write the fraction in decimal, or scale the integer",
                );
                self.cursor.bump_byte();
                let mut ignored = String::new();
                self.scan_decimal_digits(&mut ignored, &mut had_error, false);
            }
        } else {
            self.scan_decimal_digits(&mut cleaned, &mut had_error, true);
            if self.decimal_dot_starts_float() {
                self.cursor.bump_byte();
                cleaned.push('.');
                is_float = true;
                if self
                    .cursor
                    .first_byte()
                    .is_some_and(|byte| byte.is_ascii_digit())
                {
                    self.scan_decimal_digits(&mut cleaned, &mut had_error, false);
                }
            }
            if self.exponent_starts() {
                is_float = true;
                self.scan_exponent(&mut cleaned, &mut had_error);
            }
        }

        let suffix = self.scan_suffix(&mut had_error);
        let float_token = is_float || suffix.is_some_and(NumericSuffix::is_float);

        if had_error {
            return TokenKind::Error;
        }

        if float_token {
            if let Some(suffix) = suffix {
                if suffix.is_integer() {
                    let span = self.span(start, self.cursor.position());
                    self.report_once(
                        &mut had_error,
                        codes::E0008,
                        span,
                        format!(
                            "suffix `{}` is not valid on a floating-point literal",
                            suffix.as_str()
                        ),
                        "a float literal accepts `f32` or `f64`",
                    );
                    return TokenKind::Error;
                }
            }
            let mut text = cleaned;
            if text.ends_with('.') {
                text.push('0');
            }
            let value = match text.parse::<f64>() {
                Ok(value) if value.is_finite() => value,
                _ => {
                    let span = self.span(start, self.cursor.position());
                    self.report_once(
                        &mut had_error,
                        codes::E0009,
                        span,
                        "floating-point literal is out of range",
                        "the value must be finite in `f64`",
                    );
                    return TokenKind::Error;
                }
            };
            if suffix == Some(NumericSuffix::F32) && !(value as f32).is_finite() {
                let span = self.span(start, self.cursor.position());
                self.report_once(
                    &mut had_error,
                    codes::E0009,
                    span,
                    "floating-point literal is out of range for `f32`",
                    "remove the suffix or use a smaller value",
                );
                return TokenKind::Error;
            }
            TokenKind::Float { value, suffix }
        } else {
            let value = match u128::from_str_radix(&cleaned, base.radix()) {
                Ok(value) => value,
                Err(_) => {
                    let span = self.span(start, self.cursor.position());
                    self.report_once(
                        &mut had_error,
                        codes::E0009,
                        span,
                        "integer literal is too large for any integer type",
                        "the largest literal is `340282366920938463463374607431768211455`",
                    );
                    return TokenKind::Error;
                }
            };
            TokenKind::Int {
                value,
                base,
                suffix,
            }
        }
    }

    /// Consumes a `0b`/`0o`/`0x` prefix and returns its base.
    fn base_prefix(&mut self, start: usize, had_error: &mut bool) -> Option<NumericBase> {
        if self.cursor.first_byte() != Some(b'0') {
            return None;
        }
        let second = self.cursor.second_byte()?;
        let base = match second.to_ascii_lowercase() {
            b'b' => NumericBase::Binary,
            b'o' => NumericBase::Octal,
            b'x' => NumericBase::Hexadecimal,
            _ => return None,
        };
        if second.is_ascii_uppercase() {
            let span = self.span(start, start + 2);
            self.report_once(
                had_error,
                codes::E0007,
                span,
                "numeric base prefixes are lowercase",
                "write `0b`, `0o`, or `0x`",
            );
        }
        self.cursor.bump_byte();
        self.cursor.bump_byte();
        Some(base)
    }

    fn decimal_dot_starts_float(&self) -> bool {
        if self.cursor.first_byte() != Some(b'.') {
            return false;
        }
        match self.cursor.nth_char(1) {
            None => true,
            Some('.') | Some('_') => false,
            Some(character) if is_ident_start(character) => false,
            _ => true,
        }
    }

    fn exponent_starts(&self) -> bool {
        if !matches!(self.cursor.first_byte(), Some(b'e') | Some(b'E')) {
            return false;
        }
        match self.cursor.nth_char(1) {
            None | Some('_') | Some('+') | Some('-') => true,
            Some(character) if character.is_ascii_digit() => true,
            Some(character) => !seal_unicode::is_ident_continue(character),
        }
    }

    /// Scans decimal digits and separators, appending digits to `cleaned`.
    fn scan_decimal_digits(
        &mut self,
        cleaned: &mut String,
        had_error: &mut bool,
        require_digit: bool,
    ) -> bool {
        let mut digits = false;
        let mut last_separator = None;
        while let Some(byte) = self.cursor.first_byte() {
            if byte.is_ascii_digit() {
                cleaned.push(byte as char);
                digits = true;
                last_separator = None;
                self.cursor.bump_byte();
            } else if byte == b'_' {
                let position = self.cursor.position();
                if !digits {
                    let span = self.span(position, position + 1);
                    self.report_once(
                        had_error,
                        codes::E0007,
                        span,
                        "a numeric separator must appear between two digits",
                        "remove this `_` or put a digit before it",
                    );
                }
                last_separator = Some(position);
                self.cursor.bump_byte();
            } else {
                break;
            }
        }
        if let Some(position) = last_separator {
            let span = self.span(position, position + 1);
            self.report_once(
                had_error,
                codes::E0007,
                span,
                "a numeric literal cannot end with a separator",
                "remove this `_`",
            );
        }
        if require_digit && !digits {
            let position = self.cursor.position().saturating_sub(1);
            let span = self.span(position, self.cursor.position());
            self.report_once(
                had_error,
                codes::E0007,
                span,
                "numeric literal has no digits",
                "write at least one decimal digit",
            );
        }
        digits
    }

    fn scan_radix_digits(
        &mut self,
        base: NumericBase,
        cleaned: &mut String,
        had_error: &mut bool,
    ) -> bool {
        let mut digits = false;
        let mut last_separator = None;
        let radix = base.radix();
        while let Some(byte) = self.cursor.first_byte() {
            if byte == b'_' {
                let position = self.cursor.position();
                if !digits {
                    let span = self.span(position, position + 1);
                    self.report_once(
                        had_error,
                        codes::E0007,
                        span,
                        "a numeric separator must appear between two digits",
                        "remove this `_` or put a digit before it",
                    );
                }
                last_separator = Some(position);
                self.cursor.bump_byte();
                continue;
            }
            let Some(value) = digit_value(byte) else {
                break;
            };
            let position = self.cursor.position();
            if value >= radix {
                let span = self.span(position, position + 1);
                self.report_once(
                    had_error,
                    codes::E0007,
                    span,
                    format!("`{}` is not a valid digit for base {radix}", byte as char),
                    match base {
                        NumericBase::Binary => "binary digits are `0` and `1`",
                        NumericBase::Octal => "octal digits are `0` through `7`",
                        NumericBase::Hexadecimal => "hexadecimal digits are `0-9`, `a-f`, `A-F`",
                        NumericBase::Decimal => "decimal digits are `0` through `9`",
                    },
                );
                self.cursor.bump_byte();
                continue;
            }
            cleaned.push(byte as char);
            digits = true;
            last_separator = None;
            self.cursor.bump_byte();
        }
        if let Some(position) = last_separator {
            let span = self.span(position, position + 1);
            self.report_once(
                had_error,
                codes::E0007,
                span,
                "a numeric literal cannot end with a separator",
                "remove this `_`",
            );
        }
        digits
    }

    fn scan_exponent(&mut self, cleaned: &mut String, had_error: &mut bool) {
        let marker = self.cursor.position();
        let Some(character) = self.cursor.bump_char() else {
            return;
        };
        cleaned.push(character);
        if let Some(sign) = self.cursor.first_byte() {
            if sign == b'+' || sign == b'-' {
                cleaned.push(sign as char);
                self.cursor.bump_byte();
            }
        }

        let mut digits = false;
        let mut last_separator = None;
        while let Some(byte) = self.cursor.first_byte() {
            if byte.is_ascii_digit() {
                cleaned.push(byte as char);
                digits = true;
                last_separator = None;
                self.cursor.bump_byte();
            } else if byte == b'_' {
                let position = self.cursor.position();
                if !digits {
                    let span = self.span(position, position + 1);
                    self.report_once(
                        had_error,
                        codes::E0007,
                        span,
                        "a numeric separator must appear between two digits",
                        "remove this `_` or put a digit before it",
                    );
                }
                last_separator = Some(position);
                self.cursor.bump_byte();
            } else {
                break;
            }
        }
        if !digits {
            let span = self.span(marker, self.cursor.position());
            self.report_once(
                had_error,
                codes::E0007,
                span,
                "exponent has no digits",
                "write at least one digit after the exponent marker",
            );
            return;
        }
        if let Some(position) = last_separator {
            let span = self.span(position, position + 1);
            self.report_once(
                had_error,
                codes::E0007,
                span,
                "a numeric literal cannot end with a separator",
                "remove this `_`",
            );
        }
    }

    fn scan_suffix(&mut self, had_error: &mut bool) -> Option<NumericSuffix> {
        let first = self.cursor.first()?;
        if !is_ident_start(first) {
            return None;
        }
        let start = self.cursor.position();
        while let Some(character) = self.cursor.first() {
            if seal_unicode::is_ident_continue(character) {
                self.cursor.bump_char();
            } else {
                break;
            }
        }
        let text = self.cursor.slice(start..self.cursor.position());
        match NumericSuffix::parse(text) {
            Some(suffix) => Some(suffix),
            None => {
                let span = self.span(start, self.cursor.position());
                self.report_once(
                    had_error,
                    codes::E0008,
                    span,
                    format!("`{text}` is not a valid numeric suffix"),
                    "integer literals accept `i8` to `u128`, `isize`, and `usize`; floats accept `f32` and `f64`",
                );
                None
            }
        }
    }

    fn report_once(
        &self,
        had_error: &mut bool,
        code: seal_error_codes::ErrorCode,
        span: seal_span::Span,
        message: impl Into<String>,
        help: &str,
    ) {
        if *had_error {
            return;
        }
        *had_error = true;
        self.error(code, span, message).help(help).emit();
    }
}

fn digit_value(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some(u32::from(byte - b'0')),
        b'a'..=b'f' => Some(u32::from(byte - b'a') + 10),
        b'A'..=b'F' => Some(u32::from(byte - b'A') + 10),
        _ => None,
    }
}

use seal_error_codes as codes;
