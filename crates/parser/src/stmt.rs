//! Blocks and statements.

use seal_ast::{BlockId, StmtId};
use seal_error_codes as codes;
use seal_lexer::TokenKind;

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
            } else {
                let expr = self.parse_expr(0);
                let expr_span = self.builder.ast().expr(expr).span;
                if self.eat(TokenKind::Semicolon) {
                    let span = expr_span.cover(self.previous_span());
                    statements.push(self.builder.push_expr_stmt(span, expr));
                } else if self.at(TokenKind::RBrace) || self.is_eof() {
                    tail = Some(expr);
                    break;
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
        let span = open.cover(close);
        self.builder.push_block(span, statements, tail)
    }

    fn parse_let(&mut self) -> StmtId {
        let keyword = self.bump();
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
        self.builder.push_let(span, name, name_span, ty, init)
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
        {
            self.bump();
        }
        self.eat(TokenKind::Semicolon);
    }
}
