//! Blocks and statements.

use seal_ast::{BlockId, ExprKind, ForSource, StmtId};
use seal_error_codes as codes;
use seal_lexer::TokenKind;
use seal_span::Span;

use crate::parser::Parser;

impl Parser<'_> {
    /// Parses a braced block. A missing opening brace becomes an empty block and
    /// leaves recovery to the caller.
    pub(crate) fn parse_block(&mut self) -> BlockId {
        let open = self.current_span();
        if !self.expect(TokenKind::LBrace, "`{`") {
            let span = self.current_span();
            return self.builder.push_block(span, Vec::new(), None);
        }

        // A block owns its braces, so its statements and tail may use struct
        // literals and nested block expressions even when the block sits inside
        // an `if` condition that suppresses them.
        let allowed = self.struct_literal_allowed;
        self.struct_literal_allowed = true;

        let mut statements: Vec<StmtId> = Vec::new();
        let mut tail = None;
        while !self.at(TokenKind::RBrace) && !self.is_eof() {
            let before = self.index;
            if self.at(TokenKind::Let) {
                let statement = self.parse_let();
                statements.push(statement);
            } else if self.at(TokenKind::Return) {
                let statement = self.parse_return();
                statements.push(statement);
            } else if self.at(TokenKind::While) {
                let statement = self.parse_while();
                statements.push(statement);
            } else if self.at(TokenKind::For) {
                let statement = self.parse_for();
                statements.push(statement);
            } else if self.at(TokenKind::Break) {
                let statement = self.parse_break();
                statements.push(statement);
            } else if self.at(TokenKind::Continue) {
                let statement = self.parse_continue();
                statements.push(statement);
            } else {
                // A block-like expression in statement position stands alone: a
                // token on the next line starts a new statement, so `if c { }`
                // followed by `*r` is not a multiplication.
                let expr = if self.at(TokenKind::If) || self.at(TokenKind::Match) {
                    self.parse_prefix()
                } else {
                    self.parse_expr(0)
                };
                let expr_span = self.builder.ast().expr(expr).span;
                if self.eat(TokenKind::Semicolon) {
                    let span = expr_span.cover(self.previous_span());
                    statements.push(self.builder.push_expr_stmt(span, expr));
                } else if self.at(TokenKind::RBrace) || self.is_eof() {
                    tail = Some(expr);
                    break;
                } else if matches!(
                    self.builder.ast().expr(expr).kind,
                    ExprKind::If { .. } | ExprKind::Match { .. }
                ) {
                    // Block-like expressions are statements without a `;`.
                    statements.push(self.builder.push_expr_stmt(expr_span, expr));
                } else {
                    let found = self.describe(self.current_kind());
                    self.error(
                        codes::E1002,
                        self.current_span(),
                        format!("expected `;` after this expression, found {found}"),
                    )
                    .help("end the statement with `;`, or make it the last expression in the block")
                    .emit();
                    statements.push(self.builder.push_expr_stmt(expr_span, expr));
                    if self.index == before {
                        self.recover_statement();
                    }
                }
            }
            self.ensure_progress(before);
        }

        let close = if self.at(TokenKind::RBrace) {
            self.bump();
            self.previous_span()
        } else {
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1002,
                self.current_span(),
                format!("expected `}}` to close this block, found {found}"),
            )
            .help("add the missing `}`")
            .emit();
            self.current_span()
        };
        self.struct_literal_allowed = allowed;
        let span = open.cover(close);
        self.builder.push_block(span, statements, tail)
    }

    fn parse_let(&mut self) -> StmtId {
        let keyword = self.bump();
        let mutable = self.eat(TokenKind::Mut);
        let (name, name_span) = self.expect_ident("a binding name");
        let ty = if self.eat(TokenKind::Colon) {
            Some(self.parse_type())
        } else {
            None
        };
        let init = if self.eat(TokenKind::Eq) {
            Some(self.parse_expr(0))
        } else {
            None
        };

        if ty.is_none() && init.is_none() {
            self.error(
                codes::E1007,
                name_span.cover(self.previous_span()),
                "`let` binding needs a type or an initializer",
            )
            .help("write `let x: i32;` or `let x = value;`")
            .emit();
        }

        if !self.eat(TokenKind::Semicolon) {
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1002,
                self.current_span(),
                format!("expected `;` after the `let` binding, found {found}"),
            )
            .help("bindings are statements, so they end with `;`")
            .emit();
        }

        let span = keyword.span.cover(self.previous_span());
        self.builder
            .push_let(span, name, name_span, mutable, ty, init)
    }

    fn parse_while(&mut self) -> StmtId {
        let keyword = self.bump();
        let condition = self.parse_condition();
        let body = self.parse_block();
        let span = keyword.span.cover(self.previous_span());
        self.builder.push_while(span, condition, body)
    }

    fn parse_for(&mut self) -> StmtId {
        let keyword = self.bump();
        let pattern = self.parse_match_pattern();
        if !self.eat(TokenKind::In) {
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1002,
                self.current_span(),
                format!("expected `in` after the loop pattern, found {found}"),
            )
            .help("write `for value in iterable { ... }`")
            .emit();
        }
        let source = self.parse_for_source();
        let body = self.parse_block();
        let span = keyword.span.cover(self.previous_span());
        self.builder.push_for(span, pattern, source, body)
    }

    /// Parses the iterable of a `for` loop.
    ///
    /// A `{` after the iterable starts the loop body, not a struct literal, so
    /// struct literals in the header need parentheses exactly as in a condition.
    fn parse_for_source(&mut self) -> ForSource {
        let allowed = self.struct_literal_allowed;
        self.struct_literal_allowed = false;
        let start = self.parse_expr(0);
        let source = if self.at(TokenKind::DotDot) || self.at(TokenKind::DotDotEq) {
            let inclusive = self.eat(TokenKind::DotDotEq);
            if !inclusive {
                self.bump();
            }
            let end = if self.at(TokenKind::LBrace) || self.is_eof() {
                self.error(
                    codes::E1011,
                    self.current_span(),
                    "expected an end expression after the range operator",
                )
                .help("write `for value in start..end { ... }`")
                .emit();
                self.builder.push_expr(ExprKind::Error, self.current_span())
            } else {
                self.parse_expr(0)
            };
            ForSource::Range {
                start,
                end,
                inclusive,
            }
        } else {
            ForSource::Expr(start)
        };
        self.struct_literal_allowed = allowed;
        source
    }

    fn parse_break(&mut self) -> StmtId {
        let span = self.finish_loop_control("break");
        self.builder.push_break(span)
    }

    fn parse_continue(&mut self) -> StmtId {
        let span = self.finish_loop_control("continue");
        self.builder.push_continue(span)
    }

    /// Consumes a `break`/`continue` keyword and its required semicolon.
    fn finish_loop_control(&mut self, keyword: &str) -> Span {
        let token = self.bump();
        if !self.eat(TokenKind::Semicolon) {
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1002,
                self.current_span(),
                format!("expected `;` after `{keyword}`, found {found}"),
            )
            .help(format!("end the `{keyword}` with `;`"))
            .emit();
        }
        token.span.cover(self.previous_span())
    }

    fn parse_return(&mut self) -> StmtId {
        let keyword = self.bump();
        let value = if self.at(TokenKind::Semicolon) || self.at(TokenKind::RBrace) || self.is_eof()
        {
            None
        } else {
            Some(self.parse_expr(0))
        };
        if !self.eat(TokenKind::Semicolon) {
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1002,
                self.current_span(),
                format!("expected `;` after `return`, found {found}"),
            )
            .help("end the `return` with `;`")
            .emit();
        }
        let span = keyword.span.cover(self.previous_span());
        self.builder.push_return(span, value)
    }

    /// Skips to the next statement boundary after an unrecoverable prefix.
    fn recover_statement(&mut self) {
        while !self.is_eof()
            && !self.at(TokenKind::RBrace)
            && !self.at(TokenKind::Semicolon)
            && !self.at(TokenKind::Let)
            && !self.at(TokenKind::Return)
            && !self.at(TokenKind::While)
            && !self.at(TokenKind::For)
            && !self.at(TokenKind::Break)
            && !self.at(TokenKind::Continue)
            && !self.at(TokenKind::If)
        {
            self.bump();
        }
        self.eat(TokenKind::Semicolon);
    }
}
