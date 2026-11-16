//! Expressions: Pratt parsing with explicit binding powers.
//!
//! Precedence follows the C family, with assignment lowest and right-associative:
//! `||`, `&&`, `|`, `^`, `&`, equality, comparison, shifts, additive,
//! multiplicative, prefix, postfix/call. Assignment binds to a place and is
//! validated syntactically; whether the place is mutable is checked later.

use seal_ast::{AssignOp, BinaryOp, ExprId, ExprKind, Literal, UnaryOp, UpdateOp};
use seal_error_codes as codes;
use seal_lexer::{NumericSuffix, TokenKind};
use seal_symbol::Symbol;

use crate::parser::{MAX_DEPTH, Parser, can_start_expr, is_expression_stop};

/// Binding power of assignment; lower than every binary operator.
const ASSIGN_BP: u8 = 1;
/// Binding power of prefix operators; higher than every binary operator.
const PREFIX_BP: u8 = 13;

impl Parser<'_> {
    /// Parses an expression with operators that bind at least `min_bp`.
    pub(crate) fn parse_expr(&mut self, min_bp: u8) -> ExprId {
        if self.depth >= MAX_DEPTH {
            let span = self.current_span();
            return self.depth_error_expr(span);
        }
        self.depth += 1;

        let mut lhs = self.parse_prefix();
        loop {
            if self.is_eof() {
                break;
            }
            let kind = self.current_kind();

            if self.at(TokenKind::LParen) {
                lhs = self.parse_call(lhs);
                continue;
            }

            if let Some(op) = update_op(kind) {
                let token = self.bump();
                self.check_update_target(lhs);
                let span = self.builder.ast().expr(lhs).span.cover(token.span);
                lhs = self.builder.push_expr(
                    ExprKind::Update {
                        op,
                        operand: lhs,
                        postfix: true,
                    },
                    span,
                );
                continue;
            }

            if let Some(op) = assign_op(kind) {
                if min_bp > ASSIGN_BP {
                    break;
                }
                self.bump();
                self.check_assign_target(lhs);
                let value = self.parse_expr(ASSIGN_BP);
                let span = self
                    .builder
                    .ast()
                    .expr(lhs)
                    .span
                    .cover(self.builder.ast().expr(value).span);
                lhs = self.builder.push_expr(
                    ExprKind::Assign {
                        op,
                        target: lhs,
                        value,
                    },
                    span,
                );
                continue;
            }

            if let Some((op, left_bp, right_bp)) = binary_op(kind) {
                if left_bp < min_bp {
                    break;
                }
                self.bump();
                let rhs = self.parse_expr(right_bp);
                let span = self
                    .builder
                    .ast()
                    .expr(lhs)
                    .span
                    .cover(self.builder.ast().expr(rhs).span);
                lhs = self
                    .builder
                    .push_expr(ExprKind::Binary { op, lhs, rhs }, span);
                continue;
            }

            break;
        }

        self.depth -= 1;
        lhs
    }

    fn parse_prefix(&mut self) -> ExprId {
        let token = self.current();
        match token.kind {
            TokenKind::Int {
                value,
                base,
                suffix,
            } => {
                self.bump();
                let suffix = self.suffix(suffix);
                self.builder.push_expr(
                    ExprKind::Lit(Literal::Int {
                        value,
                        base: base.radix(),
                        suffix,
                    }),
                    token.span,
                )
            }
            TokenKind::Float { value, suffix } => {
                self.bump();
                let suffix = self.suffix(suffix);
                self.builder
                    .push_expr(ExprKind::Lit(Literal::Float { value, suffix }), token.span)
            }
            TokenKind::Str(symbol) => {
                self.bump();
                self.builder
                    .push_expr(ExprKind::Lit(Literal::Str(symbol)), token.span)
            }
            TokenKind::Char(character) => {
                self.bump();
                self.builder
                    .push_expr(ExprKind::Lit(Literal::Char(character)), token.span)
            }
            TokenKind::True => {
                self.bump();
                self.builder
                    .push_expr(ExprKind::Lit(Literal::Bool(true)), token.span)
            }
            TokenKind::False => {
                self.bump();
                self.builder
                    .push_expr(ExprKind::Lit(Literal::Bool(false)), token.span)
            }
            TokenKind::Ident(symbol) => {
                self.bump();
                self.builder.push_expr(ExprKind::Path(symbol), token.span)
            }
            TokenKind::Minus => {
                self.bump();
                let operand = self.parse_expr(PREFIX_BP);
                self.unary(UnaryOp::Neg, token.span, operand)
            }
            TokenKind::Bang => {
                self.bump();
                let operand = self.parse_expr(PREFIX_BP);
                self.unary(UnaryOp::Not, token.span, operand)
            }
            TokenKind::Tilde => {
                self.bump();
                let operand = self.parse_expr(PREFIX_BP);
                self.unary(UnaryOp::BitNot, token.span, operand)
            }
            TokenKind::PlusPlus | TokenKind::MinusMinus => {
                self.bump();
                let op = if token.kind == TokenKind::PlusPlus {
                    UpdateOp::Increment
                } else {
                    UpdateOp::Decrement
                };
                let operand = self.parse_expr(PREFIX_BP);
                self.check_update_target(operand);
                let span = token.span.cover(self.builder.ast().expr(operand).span);
                self.builder.push_expr(
                    ExprKind::Update {
                        op,
                        operand,
                        postfix: false,
                    },
                    span,
                )
            }
            TokenKind::LParen => {
                self.bump();
                let inner = self.parse_expr(0);
                let _ = self.expect(TokenKind::RParen, "`)`");
                let span = token.span.cover(self.previous_span());
                self.builder.push_expr(ExprKind::Paren(inner), span)
            }
            TokenKind::Error => {
                self.bump();
                self.builder.push_expr(ExprKind::Error, token.span)
            }
            _ => {
                let found = self.describe(token.kind);
                self.error(
                    codes::E1003,
                    token.span,
                    format!("expected an expression, found {found}"),
                )
                .help("write a literal, a name, or a parenthesized expression")
                .emit();
                if !is_expression_stop(token.kind) {
                    self.bump();
                }
                self.builder.push_expr(ExprKind::Error, token.span)
            }
        }
    }

    fn parse_call(&mut self, callee: ExprId) -> ExprId {
        self.bump();
        let mut args: Vec<ExprId> = Vec::new();
        if !self.at(TokenKind::RParen) && !self.is_eof() {
            loop {
                let before = self.index;
                args.push(self.parse_expr(0));
                if self.eat(TokenKind::Comma) {
                    if self.at(TokenKind::RParen) {
                        break;
                    }
                    continue;
                }
                if self.at(TokenKind::RParen) || self.is_eof() {
                    break;
                }
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E1002,
                    self.current_span(),
                    format!("expected `,` or `)` in the argument list, found {found}"),
                )
                .help("separate arguments with `,`")
                .emit();
                if !can_start_expr(self.current_kind()) {
                    break;
                }
                self.ensure_progress(before);
            }
        }
        let _ = self.expect(TokenKind::RParen, "`)` to close the call");
        let span = self
            .builder
            .ast()
            .expr(callee)
            .span
            .cover(self.previous_span());
        self.builder
            .push_expr(ExprKind::Call { callee, args }, span)
    }

    fn unary(&mut self, op: UnaryOp, operator: seal_span::Span, operand: ExprId) -> ExprId {
        let span = operator.cover(self.builder.ast().expr(operand).span);
        self.builder
            .push_expr(ExprKind::Unary { op, operand }, span)
    }

    fn suffix(&self, suffix: Option<NumericSuffix>) -> Option<Symbol> {
        suffix.map(|suffix| self.interner.intern(suffix.as_str()))
    }

    fn check_assign_target(&mut self, target: ExprId) {
        if !self.is_assignable(target) {
            let span = self.builder.ast().expr(target).span;
            self.error(codes::E1006, span, "invalid assignment target")
                .help("assign to a variable name, not to a value")
                .emit();
        }
    }

    fn check_update_target(&mut self, target: ExprId) {
        if !self.is_assignable(target) {
            let span = self.builder.ast().expr(target).span;
            self.error(codes::E1006, span, "invalid target for `++` or `--`")
                .help("increment or decrement a variable name")
                .emit();
        }
    }

    fn is_assignable(&self, expr: ExprId) -> bool {
        match &self.builder.ast().expr(expr).kind {
            ExprKind::Path(_) => true,
            ExprKind::Paren(inner) => self.is_assignable(*inner),
            _ => false,
        }
    }
}

fn binary_op(kind: TokenKind) -> Option<(BinaryOp, u8, u8)> {
    Some(match kind {
        TokenKind::OrOr => (BinaryOp::Or, 2, 3),
        TokenKind::AndAnd => (BinaryOp::And, 3, 4),
        TokenKind::Pipe => (BinaryOp::BitOr, 4, 5),
        TokenKind::Caret => (BinaryOp::BitXor, 5, 6),
        TokenKind::Amp => (BinaryOp::BitAnd, 6, 7),
        TokenKind::EqEq => (BinaryOp::Eq, 7, 8),
        TokenKind::NotEq => (BinaryOp::Ne, 7, 8),
        TokenKind::Lt => (BinaryOp::Lt, 8, 9),
        TokenKind::Gt => (BinaryOp::Gt, 8, 9),
        TokenKind::LtEq => (BinaryOp::Le, 8, 9),
        TokenKind::GtEq => (BinaryOp::Ge, 8, 9),
        TokenKind::Shl => (BinaryOp::Shl, 9, 10),
        TokenKind::Shr => (BinaryOp::Shr, 9, 10),
        TokenKind::Plus => (BinaryOp::Add, 10, 11),
        TokenKind::Minus => (BinaryOp::Sub, 10, 11),
        TokenKind::Star => (BinaryOp::Mul, 11, 12),
        TokenKind::Slash => (BinaryOp::Div, 11, 12),
        TokenKind::Percent => (BinaryOp::Rem, 11, 12),
        _ => return None,
    })
}

fn assign_op(kind: TokenKind) -> Option<AssignOp> {
    Some(match kind {
        TokenKind::Eq => AssignOp::Assign,
        TokenKind::PlusEq => AssignOp::AddAssign,
        TokenKind::MinusEq => AssignOp::SubAssign,
        TokenKind::StarEq => AssignOp::MulAssign,
        TokenKind::SlashEq => AssignOp::DivAssign,
        TokenKind::PercentEq => AssignOp::RemAssign,
        TokenKind::ShlEq => AssignOp::ShlAssign,
        TokenKind::ShrEq => AssignOp::ShrAssign,
        TokenKind::AmpEq => AssignOp::BitAndAssign,
        TokenKind::PipeEq => AssignOp::BitOrAssign,
        TokenKind::CaretEq => AssignOp::BitXorAssign,
        _ => return None,
    })
}

fn update_op(kind: TokenKind) -> Option<UpdateOp> {
    match kind {
        TokenKind::PlusPlus => Some(UpdateOp::Increment),
        TokenKind::MinusMinus => Some(UpdateOp::Decrement),
        _ => None,
    }
}
