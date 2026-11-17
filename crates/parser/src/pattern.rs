//! `match` expressions and patterns.
//!
//! Patterns are parsed by a small recursive-descent grammar. `|` binds loosest,
//! so `Some(0) | Some(1)` is one alternative list, and enum field patterns recurse
//! through the same entry point.

use seal_ast::{
    ExprId, ExprKind, MatchArm, NamedPatternField, PatternId, PatternKind, PatternLiteral,
    VariantPatternFields,
};
use seal_error_codes as codes;
use seal_lexer::TokenKind;
use seal_span::Span;

use crate::parser::{MAX_DEPTH, Parser};

impl Parser<'_> {
    /// Parses `match scrutinee { pattern [if guard] => body, ... }`.
    pub(crate) fn parse_match_expr(&mut self, start: Span) -> ExprId {
        if self.depth >= MAX_DEPTH {
            return self.depth_error_expr(start);
        }
        self.depth += 1;

        let _ = self.bump();
        // The arm list starts with `{`, so `Name { ... }` in the scrutinee needs
        // parentheses, exactly like an `if` condition.
        let allowed = self.struct_literal_allowed;
        self.struct_literal_allowed = false;
        let scrutinee = self.parse_expr(0);
        self.struct_literal_allowed = allowed;

        let _ = self.expect(TokenKind::LBrace, "`{` to start the match arms");
        let mut arms = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.is_eof() {
            let before = self.index;
            let pattern = self.parse_match_pattern();
            let pattern_span = self.builder.ast().pattern(pattern).span;
            let guard = if self.eat(TokenKind::If) {
                Some(self.parse_expr(0))
            } else {
                None
            };
            let _ = self.expect(TokenKind::FatArrow, "`=>` after the match pattern");
            let body = self.parse_expr(0);
            let body_span = self.builder.ast().expr(body).span;
            let span = pattern_span.cover(body_span);
            arms.push(MatchArm {
                pattern,
                guard,
                body,
                span,
            });

            if self.eat(TokenKind::Comma) {
                continue;
            }
            if self.at(TokenKind::RBrace) || self.is_eof() {
                break;
            }
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1002,
                self.current_span(),
                format!("expected `,` between match arms, found {found}"),
            )
            .help("separate match arms with `,`")
            .emit();
            if !can_start_pattern(self.current_kind()) {
                break;
            }
            self.ensure_progress(before);
        }
        let _ = self.expect(TokenKind::RBrace, "`}` to close the match");
        if arms.is_empty() {
            self.error(
                codes::E1011,
                self.previous_span(),
                "a match expression needs at least one arm",
            )
            .help("add an arm such as `_ => value`")
            .emit();
        }

        self.depth -= 1;
        let span = start.cover(self.previous_span());
        self.builder
            .push_expr(ExprKind::Match { scrutinee, arms }, span)
    }

    /// Parses one pattern, including top-level `|` alternatives.
    pub(crate) fn parse_match_pattern(&mut self) -> PatternId {
        let start = self.current_span();
        if self.depth >= MAX_DEPTH {
            self.report_depth(start);
            if !self.is_eof() {
                self.bump();
            }
            let span = start.cover(self.previous_span());
            return self.builder.push_pattern(PatternKind::Error, span);
        }
        self.depth += 1;

        let mut alternatives = vec![self.parse_primary_pattern()];
        while self.eat(TokenKind::Pipe) {
            alternatives.push(self.parse_primary_pattern());
        }
        self.depth -= 1;

        let first = alternatives[0];
        if alternatives.len() == 1 {
            return first;
        }
        let first_span = self.builder.ast().pattern(first).span;
        let last_span = self
            .builder
            .ast()
            .pattern(*alternatives.last().expect("at least two alternatives"))
            .span;
        self.builder
            .push_pattern(PatternKind::Or(alternatives), first_span.cover(last_span))
    }

    fn parse_primary_pattern(&mut self) -> PatternId {
        let token = self.current();
        match token.kind {
            TokenKind::Ident(symbol) if self.interner.resolve(symbol) == "_" => {
                self.bump();
                self.builder.push_pattern(PatternKind::Wildcard, token.span)
            }
            TokenKind::Ident(symbol) => {
                self.bump();
                if self.eat(TokenKind::ColonColon) {
                    let (name, name_span) = self.expect_ident("a variant name after `::`");
                    let fields = self.parse_variant_pattern_fields();
                    let span = token.span.cover(self.previous_span());
                    self.builder.push_pattern(
                        PatternKind::Variant {
                            type_name: symbol,
                            type_span: token.span,
                            name,
                            name_span,
                            fields,
                        },
                        span,
                    )
                } else if self.at(TokenKind::LParen) || self.at(TokenKind::LBrace) {
                    self.error(
                        codes::E1011,
                        token.span,
                        "an enum pattern must name its enum",
                    )
                    .help("write `Enum::Variant(...)` or `Enum::Variant { ... }`")
                    .emit();
                    let fields = self.parse_variant_pattern_fields();
                    let span = token.span.cover(self.previous_span());
                    let _ = fields;
                    self.builder.push_pattern(PatternKind::Error, span)
                } else {
                    self.builder
                        .push_pattern(PatternKind::Binding(symbol), token.span)
                }
            }
            TokenKind::Int { value, suffix, .. } => {
                self.bump();
                let suffix = self.suffix(suffix);
                self.builder.push_pattern(
                    PatternKind::Literal(PatternLiteral::Int {
                        value,
                        negative: false,
                        suffix,
                    }),
                    token.span,
                )
            }
            TokenKind::Char(value) => {
                self.bump();
                self.builder.push_pattern(
                    PatternKind::Literal(PatternLiteral::Char(value)),
                    token.span,
                )
            }
            TokenKind::True | TokenKind::False => {
                self.bump();
                self.builder.push_pattern(
                    PatternKind::Literal(PatternLiteral::Bool(token.kind == TokenKind::True)),
                    token.span,
                )
            }
            TokenKind::Minus => {
                self.bump();
                let next = self.current();
                match next.kind {
                    TokenKind::Int { value, suffix, .. } => {
                        self.bump();
                        let suffix = self.suffix(suffix);
                        let span = token.span.cover(next.span);
                        self.builder.push_pattern(
                            PatternKind::Literal(PatternLiteral::Int {
                                value,
                                negative: true,
                                suffix,
                            }),
                            span,
                        )
                    }
                    TokenKind::Float { value, suffix } => {
                        self.bump();
                        let suffix = self.suffix(suffix);
                        let span = token.span.cover(next.span);
                        self.builder.push_pattern(
                            PatternKind::Literal(PatternLiteral::Float {
                                value: -value,
                                suffix,
                            }),
                            span,
                        )
                    }
                    _ => {
                        let found = self.describe(next.kind);
                        self.error(
                            codes::E1011,
                            token.span.cover(next.span),
                            format!("expected a number after `-`, found {found}"),
                        )
                        .help("write a numeric literal pattern, such as `-1`")
                        .emit();
                        if !self.is_eof() {
                            self.bump();
                        }
                        self.builder.push_pattern(
                            PatternKind::Error,
                            token.span.cover(self.previous_span()),
                        )
                    }
                }
            }
            TokenKind::Float { value, suffix } => {
                self.bump();
                let suffix = self.suffix(suffix);
                self.builder.push_pattern(
                    PatternKind::Literal(PatternLiteral::Float { value, suffix }),
                    token.span,
                )
            }
            TokenKind::LParen => {
                self.bump();
                let inner = self.parse_match_pattern();
                let _ = self.expect(TokenKind::RParen, "`)` after the pattern");
                inner
            }
            TokenKind::Str(_) => {
                self.error(
                    codes::E1011,
                    token.span,
                    "a string cannot be used as a pattern literal",
                )
                .help("bind the value with a name or use `_`")
                .emit();
                self.bump();
                self.builder.push_pattern(PatternKind::Error, token.span)
            }
            _ => {
                let found = self.describe(token.kind);
                self.error(
                    codes::E1011,
                    token.span,
                    format!("expected a pattern, found {found}"),
                )
                .help("write `_`, a binding name, a literal, or `Enum::Variant(...)`")
                .emit();
                if !is_pattern_stop(token.kind) {
                    self.bump();
                }
                self.builder.push_pattern(PatternKind::Error, token.span)
            }
        }
    }

    /// Parses the optional field list after a variant name.
    fn parse_variant_pattern_fields(&mut self) -> VariantPatternFields {
        if self.at(TokenKind::LParen) {
            self.bump();
            let mut patterns = Vec::new();
            while !self.at(TokenKind::RParen) && !self.is_eof() {
                let before = self.index;
                patterns.push(self.parse_match_pattern());
                if self.eat(TokenKind::Comma) {
                    continue;
                }
                if self.at(TokenKind::RParen) || self.is_eof() {
                    break;
                }
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E1002,
                    self.current_span(),
                    format!("expected `,` or `)` in the pattern fields, found {found}"),
                )
                .help("separate field patterns with `,`")
                .emit();
                if !can_start_pattern(self.current_kind()) {
                    break;
                }
                self.ensure_progress(before);
            }
            let _ = self.expect(TokenKind::RParen, "`)` to close the pattern fields");
            return VariantPatternFields::Positional(patterns);
        }

        if self.at(TokenKind::LBrace) {
            self.bump();
            let mut fields = Vec::new();
            while !self.at(TokenKind::RBrace) && !self.is_eof() {
                let before = self.index;
                let (name, name_span) = self.expect_ident("a field name");
                let pattern = if self.eat(TokenKind::Colon) {
                    self.parse_match_pattern()
                } else {
                    self.builder
                        .push_pattern(PatternKind::Binding(name), name_span)
                };
                let pattern_span = self.builder.ast().pattern(pattern).span;
                fields.push(NamedPatternField {
                    name,
                    name_span,
                    pattern,
                    span: name_span.cover(pattern_span),
                });
                if self.eat(TokenKind::Comma) {
                    continue;
                }
                if self.at(TokenKind::RBrace) || self.is_eof() {
                    break;
                }
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E1002,
                    self.current_span(),
                    format!("expected `,` or `}}` in the pattern fields, found {found}"),
                )
                .help("separate field patterns with `,`")
                .emit();
                if !can_start_pattern(self.current_kind()) {
                    break;
                }
                self.ensure_progress(before);
            }
            let _ = self.expect(TokenKind::RBrace, "`}` to close the pattern fields");
            return VariantPatternFields::Named(fields);
        }

        VariantPatternFields::Unit
    }
}

fn is_pattern_stop(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Eof
            | TokenKind::FatArrow
            | TokenKind::Comma
            | TokenKind::RParen
            | TokenKind::RBrace
    )
}

fn can_start_pattern(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Ident(_)
            | TokenKind::Int { .. }
            | TokenKind::Float { .. }
            | TokenKind::Char(_)
            | TokenKind::True
            | TokenKind::False
            | TokenKind::Minus
            | TokenKind::LParen
            | TokenKind::Str(_)
            | TokenKind::Error
    )
}
