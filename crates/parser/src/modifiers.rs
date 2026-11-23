//! Contextual modifier parsing.
//!
//! `public`, `internal`, `private`, `cfg`, `test`, and `deprecated` are ordinary
//! identifiers outside the prefix of an item, field, or method. The parser only
//! consumes them in that position, so existing code that uses those names keeps
//! working.

use seal_ast::{Deprecation, Modifiers, Visibility};
use seal_error_codes as codes;
use seal_lexer::TokenKind;

use crate::parser::Parser;

impl Parser<'_> {
    /// Parses every modifier at the current position.
    ///
    /// Shape errors are reported here. Whether a modifier is meaningful for the
    /// following construct is decided by the caller, which has the grammar
    /// context to emit one specific diagnostic.
    pub(crate) fn parse_modifiers(&mut self) -> Modifiers {
        let mut modifiers = Modifiers::none();
        let mut explicit_visibility: Option<Visibility> = None;

        loop {
            let visibility = if self.at_name("public") {
                Some(Visibility::Public)
            } else if self.at_name("internal") {
                Some(Visibility::Internal)
            } else if self.at_name("private") {
                Some(Visibility::Private)
            } else {
                None
            };
            if let Some(visibility) = visibility {
                let span = self.bump().span;
                if explicit_visibility.replace(visibility).is_some() {
                    self.error(
                        codes::E1001,
                        span,
                        "an item has only one visibility modifier",
                    )
                    .help("keep one of `public`, `internal`, or `private`")
                    .emit();
                } else {
                    modifiers.visibility = visibility;
                    modifiers.has_visibility = true;
                }
                continue;
            }

            if self.at_name("cfg") {
                let span = self.bump().span;
                if modifiers.cfg.is_some() {
                    self.error(codes::E1001, span, "an item has only one `cfg` condition")
                        .help("combine the conditions with `&&` or `||` inside one `cfg(...)`")
                        .emit();
                } else {
                    modifiers.cfg = Some(self.parse_cfg_expr(span));
                }
                continue;
            }

            if self.at_name("export") {
                let span = self.current_span();
                let Some(TokenKind::Str(abi)) = self.peek_kind() else {
                    break;
                };
                self.bump();
                self.bump();
                if modifiers.export_span.is_some() {
                    self.error(codes::E1001, span, "an item has only one `export` ABI")
                        .help("remove the duplicate `export` modifier")
                        .emit();
                } else {
                    modifiers.export = Some(abi);
                    modifiers.export_span = Some(span);
                }
                continue;
            }

            if self.at_name("deprecated") && matches!(self.peek_kind(), Some(TokenKind::LParen)) {
                let span = self.current_span();
                let parsed = self.parse_deprecated();
                if modifiers.deprecated.is_some() {
                    self.error(
                        codes::E1001,
                        span,
                        "an item has only one `deprecated` modifier",
                    )
                    .help("remove the duplicate `deprecated` modifier")
                    .emit();
                } else {
                    modifiers.deprecated = parsed;
                }
                continue;
            }

            if self.at_name("test") {
                let span = self.bump().span;
                if modifiers.is_test {
                    self.error(codes::E1001, span, "`test` is written only once")
                        .help("remove the duplicate modifier")
                        .emit();
                } else {
                    modifiers.is_test = true;
                }
                continue;
            }

            if self.at_name("bench") {
                let span = self.bump().span;
                if modifiers.is_bench {
                    self.error(codes::E1001, span, "`bench` is written only once")
                        .help("remove the duplicate modifier")
                        .emit();
                } else {
                    modifiers.is_bench = true;
                }
                continue;
            }

            break;
        }

        if modifiers.is_test && modifiers.is_bench {
            self.error(
                codes::E1001,
                self.current_span(),
                "a function cannot be both a test and a benchmark",
            )
            .help("keep `test` or `bench`, not both")
            .emit();
            modifiers.is_bench = false;
        }

        modifiers
    }

    /// Parses `deprecated(since: "…", note: "…", replacement: "…")`.
    ///
    /// `since` is required and must be a semantic version. The other entries
    /// are optional. Every value is untrusted source text: empty values,
    /// control characters, and overlong notes are rejected before the
    /// deprecation reaches a definition.
    pub(crate) fn parse_deprecated(&mut self) -> Option<Deprecation> {
        let start = self.bump().span;
        if !self.expect(TokenKind::LParen, "`(` after `deprecated`") {
            return None;
        }

        let mut since = None;
        let mut saw_since = false;
        let mut note = None;
        let mut replacement = None;
        let mut valid = true;

        while !self.at(TokenKind::RParen) && !self.is_eof() {
            let before = self.index;
            let (key, key_span) = self.expect_ident("a deprecation entry");
            let key_name = self.interner.resolve(key).to_owned();
            if !self.expect(TokenKind::Colon, "`:` after the deprecation entry") {
                valid = false;
                self.recover_deprecation_entry();
                self.ensure_progress(before);
                continue;
            }
            let token = self.current();
            let TokenKind::Str(value) = token.kind else {
                let found = self.describe(token.kind);
                self.error(
                    codes::E1001,
                    token.span,
                    format!(
                        "the deprecation entry `{key_name}` needs a string value, found {found}"
                    ),
                )
                .help("write the value in quotes, such as `since: \"0.2.0\"`")
                .emit();
                valid = false;
                self.recover_deprecation_entry();
                self.ensure_progress(before);
                continue;
            };
            self.bump();
            let text = self.interner.resolve(value);
            let text_valid = self.validate_deprecation_text(&key_name, text, token.span);
            valid &= text_valid;

            match key_name.as_str() {
                "since" => {
                    if saw_since {
                        self.error(
                            codes::E1001,
                            key_span,
                            "the deprecation entry `since` is written twice",
                        )
                        .help("keep one `since` release")
                        .emit();
                        valid = false;
                    } else {
                        saw_since = true;
                        if text_valid {
                            if let Err(error) = semver::Version::parse(text) {
                                self.error(
                                    codes::E1001,
                                    token.span,
                                    format!(
                                        "the deprecation `since` value `{text}` is not a semantic version: {error}"
                                    ),
                                )
                                .help("use a tri-part version such as `0.2.0`")
                                .emit();
                                valid = false;
                            } else {
                                since = Some(value);
                            }
                        }
                    }
                }
                "note" => {
                    if note.is_some() {
                        self.error(
                            codes::E1001,
                            key_span,
                            "the deprecation entry `note` is written twice",
                        )
                        .help("combine the guidance into one note")
                        .emit();
                        valid = false;
                    } else if text_valid {
                        note = Some(value);
                    }
                }
                "replacement" => {
                    if replacement.is_some() {
                        self.error(
                            codes::E1001,
                            key_span,
                            "the deprecation entry `replacement` is written twice",
                        )
                        .help("keep one replacement name")
                        .emit();
                        valid = false;
                    } else if text_valid {
                        replacement = Some(value);
                    }
                }
                _ => {
                    self.error(
                        codes::E1001,
                        key_span,
                        format!(
                            "`{key_name}` is not a deprecation entry; expected `since`, `note`, or `replacement`"
                        ),
                    )
                    .help("remove the unknown entry")
                    .emit();
                    valid = false;
                }
            }

            if self.eat(TokenKind::Comma) {
                continue;
            }
            if !self.at(TokenKind::RParen) {
                let span = self.current_span();
                self.error(
                    codes::E1001,
                    span,
                    "deprecation entries are separated by commas",
                )
                .help("add a comma before the next entry")
                .emit();
                valid = false;
                self.recover_deprecation_entry();
            }
            self.ensure_progress(before);
        }

        let _ = self.expect(TokenKind::RParen, "`)` to close `deprecated(...)`");
        if !saw_since {
            self.error(
                codes::E1001,
                start,
                "`deprecated(...)` requires a `since` release",
            )
            .help("add `since: \"x.y.z\"` naming the release that deprecated the item")
            .emit();
            valid = false;
        }

        if !valid {
            return None;
        }
        Some(Deprecation {
            since: since?,
            note,
            replacement,
            span: start.cover(self.previous_span()),
        })
    }

    fn validate_deprecation_text(&self, key: &str, text: &str, span: seal_span::Span) -> bool {
        let mut valid = true;
        if text.is_empty() {
            self.error(
                codes::E1001,
                span,
                format!("the deprecation `{key}` cannot be empty"),
            )
            .help("provide a non-empty value")
            .emit();
            valid = false;
        }
        if text.len() > 256 {
            self.error(
                codes::E1001,
                span,
                format!("the deprecation `{key}` must be at most 256 bytes"),
            )
            .help("shorten the value")
            .emit();
            valid = false;
        }
        if text.chars().any(char::is_control) {
            self.error(
                codes::E1001,
                span,
                format!("the deprecation `{key}` cannot contain control characters"),
            )
            .help("remove the control characters")
            .emit();
            valid = false;
        }
        valid
    }

    fn recover_deprecation_entry(&mut self) {
        while !self.is_eof() && !self.at(TokenKind::RParen) && !self.at(TokenKind::Comma) {
            self.bump();
        }
        let _ = self.eat(TokenKind::Comma);
    }
}
