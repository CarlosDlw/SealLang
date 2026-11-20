//! Parsing for contextual `cfg(...)` conditions.
//!
//! The grammar is deliberately small and typed: a condition is a boolean
//! expression over known keys and literals, with `!` binding tighter than
//! `&&`, which binds tighter than `||`. Every malformed shape produces one
//! diagnostic and leaves the parser at a point where the next item can start.

use seal_ast::{CfgCmpOp, CfgExprId, CfgExprKind, CfgLiteral};
use seal_error_codes as codes;
use seal_lexer::TokenKind;
use seal_span::Span;

use crate::parser::Parser;

impl Parser<'_> {
    /// Parses the parenthesized expression after the contextual `cfg` name.
    pub(crate) fn parse_cfg_expr(&mut self, cfg_span: Span) -> CfgExprId {
        if !self.eat(TokenKind::LParen) {
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1002,
                self.current_span(),
                format!("expected `(` after `cfg`, found {found}"),
            )
            .help("write the condition as `cfg(key)` or `cfg(key == value)`")
            .emit();
            return self.builder.push_cfg_expr(CfgExprKind::Error, cfg_span);
        }

        let expression = self.parse_cfg_or();
        if self.at(TokenKind::RParen) {
            self.bump();
        } else {
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1002,
                self.current_span(),
                format!("expected `)` to close `cfg(`, found {found}"),
            )
            .help("close the condition before the next item")
            .emit();
            self.skip_cfg_tail();
        }
        expression
    }

    fn parse_cfg_or(&mut self) -> CfgExprId {
        let mut left = self.parse_cfg_and();
        while self.eat(TokenKind::OrOr) {
            let right = self.parse_cfg_and();
            let span = self
                .builder
                .cfg_expr(left)
                .span
                .cover(self.builder.cfg_expr(right).span);
            left = self.builder.push_cfg_expr(
                CfgExprKind::Or {
                    lhs: left,
                    rhs: right,
                },
                span,
            );
        }
        left
    }

    fn parse_cfg_and(&mut self) -> CfgExprId {
        let mut left = self.parse_cfg_unary();
        while self.eat(TokenKind::AndAnd) {
            let right = self.parse_cfg_unary();
            let span = self
                .builder
                .cfg_expr(left)
                .span
                .cover(self.builder.cfg_expr(right).span);
            left = self.builder.push_cfg_expr(
                CfgExprKind::And {
                    lhs: left,
                    rhs: right,
                },
                span,
            );
        }
        left
    }

    fn parse_cfg_unary(&mut self) -> CfgExprId {
        if self.at(TokenKind::Bang) {
            let start = self.bump().span;
            let operand = self.parse_cfg_unary();
            let span = start.cover(self.builder.cfg_expr(operand).span);
            return self.builder.push_cfg_expr(CfgExprKind::Not(operand), span);
        }
        if self.at(TokenKind::LParen) {
            self.bump();
            let inner = self.parse_cfg_or();
            if !self.eat(TokenKind::RParen) {
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E1002,
                    self.current_span(),
                    format!("expected `)` after a parenthesized condition, found {found}"),
                )
                .emit();
                self.skip_cfg_tail();
            }
            return inner;
        }
        let atom = self.parse_cfg_atom();
        self.parse_cfg_comparison(atom)
    }

    fn parse_cfg_atom(&mut self) -> CfgExprId {
        let start = self.current_span();
        match self.current_kind() {
            TokenKind::Str(_) => {
                self.error(
                    codes::E2084,
                    start,
                    "a string literal is not a configuration key",
                )
                .help("write a key such as `target.os == \"linux\"` or `feature(\"std\")`")
                .emit();
                self.bump();
                self.builder.push_cfg_expr(CfgExprKind::Error, start)
            }
            TokenKind::Int { value, .. } => {
                self.bump();
                self.builder
                    .push_cfg_expr(CfgExprKind::Literal(CfgLiteral::Int(value)), start)
            }
            TokenKind::True => {
                self.bump();
                self.builder
                    .push_cfg_expr(CfgExprKind::Literal(CfgLiteral::Bool(true)), start)
            }
            TokenKind::False => {
                self.bump();
                self.builder
                    .push_cfg_expr(CfgExprKind::Literal(CfgLiteral::Bool(false)), start)
            }
            TokenKind::Ident(name) => {
                self.bump();
                let spelling = self.interner.resolve(name);
                match spelling {
                    "feature" => self.parse_cfg_string_call(start, "feature", true),
                    "env" => self.parse_cfg_string_call(start, "env", true),
                    "target" => self.parse_cfg_target(start),
                    "debug_assertions" => self
                        .builder
                        .push_cfg_expr(CfgExprKind::DebugAssertions, start),
                    "test" => self.builder.push_cfg_expr(CfgExprKind::Test, start),
                    "bench" => self.builder.push_cfg_expr(CfgExprKind::Bench, start),
                    _ => {
                        self.error(
                            codes::E2084,
                            start,
                            format!("unknown configuration key `{spelling}`"),
                        )
                        .help(
                            "use `feature(\"name\")`, `env(\"NAME\")`, `target.<key>`, \
                             `debug_assertions`, `test`, or `bench`",
                        )
                        .emit();
                        self.builder.push_cfg_expr(CfgExprKind::Error, start)
                    }
                }
            }
            _ => {
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E2084,
                    start,
                    format!("expected a configuration key, found {found}"),
                )
                .emit();
                if !self.at(TokenKind::RParen) && !self.is_eof() {
                    self.bump();
                }
                self.builder.push_cfg_expr(CfgExprKind::Error, start)
            }
        }
    }

    /// Parses `feature("name")` or `env("NAME")`.
    fn parse_cfg_string_call(&mut self, start: Span, name: &str, one_argument: bool) -> CfgExprId {
        if !self.eat(TokenKind::LParen) {
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1002,
                self.current_span(),
                format!("expected `(` after `{name}`, found {found}"),
            )
            .help(format!("write `{name}(\"value\")`"))
            .emit();
            return self.builder.push_cfg_expr(CfgExprKind::Error, start);
        }

        let value = match self.current_kind() {
            TokenKind::Str(value) => {
                self.bump();
                Some(value)
            }
            _ => {
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E1002,
                    self.current_span(),
                    format!("expected a string literal in `{name}(...)`, found {found}"),
                )
                .emit();
                None
            }
        };

        if one_argument && self.at(TokenKind::Comma) {
            self.error(
                codes::E1002,
                self.current_span(),
                format!("`{name}` takes exactly one string argument"),
            )
            .help("remove the extra arguments")
            .emit();
            self.skip_cfg_arguments();
        }

        if !self.eat(TokenKind::RParen) {
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1002,
                self.current_span(),
                format!("expected `)` after `{name}(...)`, found {found}"),
            )
            .emit();
            self.skip_cfg_tail();
        }

        let span = start.cover(self.previous_span());
        match value {
            Some(value) if name == "feature" => self
                .builder
                .push_cfg_expr(CfgExprKind::Feature(value), span),
            Some(value) => self.builder.push_cfg_expr(CfgExprKind::Env(value), span),
            None => self.builder.push_cfg_expr(CfgExprKind::Error, span),
        }
    }

    /// Parses `target.<os|arch|env|pointer_width>`.
    fn parse_cfg_target(&mut self, start: Span) -> CfgExprId {
        if !self.eat(TokenKind::Dot) {
            let found = self.describe(self.current_kind());
            self.error(
                codes::E2084,
                self.current_span(),
                format!("expected `.` after `target`, found {found}"),
            )
            .help("write `target.os`, `target.arch`, `target.env`, or `target.pointer_width`")
            .emit();
            return self.builder.push_cfg_expr(CfgExprKind::Error, start);
        }

        let key = match self.current_kind() {
            TokenKind::Ident(key) => {
                self.bump();
                key
            }
            _ => {
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E2084,
                    self.current_span(),
                    format!("expected a target key, found {found}"),
                )
                .help("write `target.os`, `target.arch`, `target.env`, or `target.pointer_width`")
                .emit();
                return self.builder.push_cfg_expr(CfgExprKind::Error, start);
            }
        };

        let spelling = self.interner.resolve(key);
        if !matches!(spelling, "os" | "arch" | "env" | "pointer_width") {
            self.error(
                codes::E2084,
                start.cover(self.previous_span()),
                format!("unknown target key `{spelling}`"),
            )
            .help("use `target.os`, `target.arch`, `target.env`, or `target.pointer_width`")
            .emit();
            return self.builder.push_cfg_expr(CfgExprKind::Error, start);
        }

        let span = start.cover(self.previous_span());
        self.builder.push_cfg_expr(CfgExprKind::Target(key), span)
    }

    /// Parses a comparison after an atom, if one is present.
    fn parse_cfg_comparison(&mut self, left: CfgExprId) -> CfgExprId {
        let op = if self.at(TokenKind::EqEq) {
            Some(CfgCmpOp::Eq)
        } else if self.at(TokenKind::NotEq) {
            Some(CfgCmpOp::Ne)
        } else {
            None
        };
        let Some(op) = op else {
            return left;
        };
        self.bump();
        let right = self.parse_cfg_value();
        let span = self
            .builder
            .cfg_expr(left)
            .span
            .cover(self.builder.cfg_expr(right).span);
        self.builder.push_cfg_expr(
            CfgExprKind::Compare {
                lhs: left,
                op,
                rhs: right,
            },
            span,
        )
    }

    fn parse_cfg_value(&mut self) -> CfgExprId {
        let start = self.current_span();
        let literal = match self.current_kind() {
            TokenKind::Str(value) => {
                self.bump();
                Some(CfgLiteral::Str(value))
            }
            TokenKind::Int { value, .. } => {
                self.bump();
                Some(CfgLiteral::Int(value))
            }
            TokenKind::True => {
                self.bump();
                Some(CfgLiteral::Bool(true))
            }
            TokenKind::False => {
                self.bump();
                Some(CfgLiteral::Bool(false))
            }
            _ => None,
        };
        match literal {
            Some(literal) => self
                .builder
                .push_cfg_expr(CfgExprKind::Literal(literal), start),
            None => {
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E2084,
                    start,
                    format!("expected a literal on the right of a `cfg` comparison, found {found}"),
                )
                .help("compare against a string, boolean, or integer literal")
                .emit();
                self.builder.push_cfg_expr(CfgExprKind::Error, start)
            }
        }
    }

    /// Consumes the rest of a malformed `cfg(...)` up to its closing parenthesis
    /// or the start of the next item.
    fn skip_cfg_tail(&mut self) {
        while !self.is_eof() && !self.at(TokenKind::RParen) && !self.cfg_item_boundary() {
            self.bump();
        }
        if self.at(TokenKind::RParen) {
            self.bump();
        }
    }

    fn skip_cfg_arguments(&mut self) {
        while !self.is_eof() && !self.at(TokenKind::RParen) {
            self.bump();
        }
    }

    fn cfg_item_boundary(&self) -> bool {
        matches!(
            self.current_kind(),
            TokenKind::Func
                | TokenKind::Unsafe
                | TokenKind::Const
                | TokenKind::Static
                | TokenKind::Struct
                | TokenKind::Enum
                | TokenKind::Impl
                | TokenKind::Trait
        ) || self.at_name("extern")
            || self.at_name("use")
            || self.at_name("module")
            || self.at_name("public")
            || self.at_name("internal")
            || self.at_name("private")
            || self.at_name("cfg")
            || self.at_name("test")
    }
}
