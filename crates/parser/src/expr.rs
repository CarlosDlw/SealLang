//! Expressions: Pratt parsing with explicit binding powers.
//!
//! Precedence follows the C family, with assignment lowest and right-associative:
//! `||`, `&&`, `|`, `^`, `&`, equality, comparison, shifts, additive,
//! multiplicative, prefix, postfix/call. Assignment binds to a place and is
//! validated syntactically; whether the place is mutable is checked later.
//!
//! Prefix `&` and `*` are borrow and dereference, not the binary operators with
//! the same spelling: prefix position decides, so `&x & y` is `(&x) & y`.

use seal_ast::{AssignOp, BinaryOp, ExprId, ExprKind, FieldInit, Literal, UnaryOp, UpdateOp};
use seal_error_codes as codes;
use seal_lexer::{NumericSuffix, TokenKind};
use seal_symbol::Symbol;

use crate::parser::{MAX_DEPTH, Parser, can_start_expr, is_expression_stop};

/// Binding power of assignment; lower than every binary operator.
const ASSIGN_BP: u8 = 1;
/// Binding power of prefix operators; higher than every binary operator.
const PREFIX_BP: u8 = 13;

/// Which braced initializer list is being parsed, for diagnostic wording.
#[derive(Clone, Copy)]
enum InitializerContext {
    Struct,
    Variant,
}

impl InitializerContext {
    fn description(self) -> &'static str {
        match self {
            Self::Struct => "struct literal",
            Self::Variant => "variant literal",
        }
    }

    fn closing(self) -> &'static str {
        match self {
            Self::Struct => "`}` to close the struct literal",
            Self::Variant => "`}` to close the variant literal",
        }
    }
}

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

            if self.at(TokenKind::LBracket) {
                lhs = self.parse_index(lhs);
                continue;
            }

            if self.at(TokenKind::Dot) {
                self.bump();
                let (name, name_span) = self.expect_ident("a field name after `.`");
                if self.at(TokenKind::LParen) {
                    let (args, end) = self.parse_call_arguments();
                    let span = self.builder.ast().expr(lhs).span.cover(end);
                    lhs = self.builder.push_expr(
                        ExprKind::MethodCall {
                            receiver: lhs,
                            name,
                            name_span,
                            args,
                        },
                        span,
                    );
                } else {
                    let span = self
                        .builder
                        .ast()
                        .expr(lhs)
                        .span
                        .cover(self.previous_span());
                    lhs = self.builder.push_expr(
                        ExprKind::Field {
                            base: lhs,
                            name,
                            name_span,
                        },
                        span,
                    );
                }
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

    pub(crate) fn parse_prefix(&mut self) -> ExprId {
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
                if self.eat(TokenKind::ColonColon) {
                    let (name, name_span) = self.expect_ident("an associated name after `::`");
                    if self.struct_literal_allowed && self.at(TokenKind::LBrace) {
                        self.parse_variant_literal(symbol, token.span, name, name_span)
                    } else {
                        let span = token.span.cover(name_span);
                        self.builder.push_expr(
                            ExprKind::PathQualified {
                                type_name: symbol,
                                type_span: token.span,
                                name,
                                name_span,
                            },
                            span,
                        )
                    }
                } else if self.struct_literal_allowed && self.at(TokenKind::LBrace) {
                    self.parse_struct_literal(symbol, token.span)
                } else {
                    self.builder.push_expr(ExprKind::Path(symbol), token.span)
                }
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
            TokenKind::Amp => {
                self.bump();
                let mutable = self.eat(TokenKind::Mut);
                let operand = self.parse_expr(PREFIX_BP);
                let span = token.span.cover(self.builder.ast().expr(operand).span);
                self.builder
                    .push_expr(ExprKind::Borrow { operand, mutable }, span)
            }
            TokenKind::Star => {
                self.bump();
                let operand = self.parse_expr(PREFIX_BP);
                let span = token.span.cover(self.builder.ast().expr(operand).span);
                self.builder.push_expr(ExprKind::Deref { operand }, span)
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
                let allowed = self.struct_literal_allowed;
                self.struct_literal_allowed = true;
                self.bump();
                let inner = self.parse_expr(0);
                self.struct_literal_allowed = allowed;
                let _ = self.expect(TokenKind::RParen, "`)`");
                let span = token.span.cover(self.previous_span());
                self.builder.push_expr(ExprKind::Paren(inner), span)
            }
            TokenKind::LBracket => self.parse_array_literal(token.span),
            TokenKind::If => self.parse_if_expr(token.span),
            TokenKind::Match => self.parse_match_expr(token.span),
            // A `{` starts a block expression unless a surrounding construct
            // owns the braces: `if`/`while` conditions and a match scrutinee
            // share the struct-literal flag, because `{` there starts the body
            // or the arm list.
            TokenKind::LBrace if self.struct_literal_allowed => {
                let block = self.parse_block();
                let span = token.span.cover(self.previous_span());
                self.builder.push_expr(ExprKind::Block(block), span)
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

    /// Parses `if condition block ("else" (block | if ...))?`.
    ///
    /// `else if` becomes a synthetic block whose tail is the nested `if`, so
    /// every branch is a block. The recursion for `else if` is bounded by the
    /// same depth limit as every other nested construct.
    fn parse_if_expr(&mut self, start: seal_span::Span) -> ExprId {
        if self.depth >= MAX_DEPTH {
            return self.depth_error_expr(start);
        }
        self.depth += 1;

        let _ = self.bump();
        let condition = self.parse_condition();
        let then_branch = self.parse_block();
        let else_branch = if self.eat(TokenKind::Else) {
            if self.at(TokenKind::If) {
                let nested_start = self.current_span();
                let nested = self.parse_if_expr(nested_start);
                let nested_span = self.builder.ast().expr(nested).span;
                Some(
                    self.builder
                        .push_block(nested_span, Vec::new(), Some(nested)),
                )
            } else if self.at(TokenKind::LBrace) {
                Some(self.parse_block())
            } else {
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E1002,
                    self.current_span(),
                    format!("expected `{{` or `if` after `else`, found {found}"),
                )
                .help("write `else { ... }` or `else if ...`")
                .emit();
                let span = self.current_span();
                Some(self.builder.push_block(span, Vec::new(), None))
            }
        } else {
            None
        };

        self.depth -= 1;
        let span = start.cover(self.previous_span());
        self.builder
            .push_if(span, condition, then_branch, else_branch)
    }

    /// Parses a condition, where a `{` starts the body rather than a struct
    /// literal. Parentheses re-enable literals inside the condition.
    pub(crate) fn parse_condition(&mut self) -> ExprId {
        let allowed = self.struct_literal_allowed;
        self.struct_literal_allowed = false;
        let condition = self.parse_expr(0);
        self.struct_literal_allowed = allowed;
        condition
    }

    fn parse_struct_literal(&mut self, name: Symbol, start: seal_span::Span) -> ExprId {
        let fields = self.parse_field_initializers(InitializerContext::Struct);
        let span = start.cover(self.previous_span());
        self.builder.push_expr(
            ExprKind::Struct {
                name,
                name_span: start,
                fields,
            },
            span,
        )
    }

    /// Parses a qualified variant literal after its name.
    fn parse_variant_literal(
        &mut self,
        type_name: Symbol,
        type_span: seal_span::Span,
        name: Symbol,
        name_span: seal_span::Span,
    ) -> ExprId {
        let fields = self.parse_field_initializers(InitializerContext::Variant);
        let span = type_span.cover(self.previous_span());
        self.builder.push_expr(
            ExprKind::Variant {
                type_name,
                type_span,
                name,
                name_span,
                fields,
            },
            span,
        )
    }

    /// Parses a braced field initializer list, starting at `{`.
    ///
    /// Struct literals and variant literals share the same grammar, so both use
    /// this routine; `context` only changes the wording of diagnostics.
    fn parse_field_initializers(&mut self, context: InitializerContext) -> Vec<FieldInit> {
        debug_assert!(self.at(TokenKind::LBrace));
        self.bump();
        let mut fields = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.is_eof() {
            let before = self.index;
            let (field_name, field_name_span) = self.expect_ident("a field name");
            let _ = self.expect(TokenKind::Colon, "`:` after the field name");
            let allowed = self.struct_literal_allowed;
            self.struct_literal_allowed = true;
            let value = self.parse_expr(0);
            self.struct_literal_allowed = allowed;
            let span = field_name_span.cover(self.builder.ast().expr(value).span);
            fields.push(FieldInit {
                name: field_name,
                name_span: field_name_span,
                value,
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
                format!(
                    "expected `,` or `}}` in the {}, found {found}",
                    context.description()
                ),
            )
            .help("separate field initializers with `,`")
            .emit();
            if !can_start_expr(self.current_kind()) {
                break;
            }
            self.ensure_progress(before);
        }
        let _ = self.expect(TokenKind::RBrace, context.closing());
        fields
    }

    /// Parses an array literal or a repeated array literal.
    ///
    /// `[]` is an empty array; `[a, b, c]` has one element per expression; and
    /// `[value; N]` evaluates `value` once and copies it into `N` elements.
    fn parse_array_literal(&mut self, start: seal_span::Span) -> ExprId {
        self.bump();
        let allowed = self.struct_literal_allowed;
        self.struct_literal_allowed = true;

        if self.eat(TokenKind::RBracket) {
            self.struct_literal_allowed = allowed;
            let span = start.cover(self.previous_span());
            return self.builder.push_expr(
                ExprKind::Array {
                    elements: Vec::new(),
                },
                span,
            );
        }

        let before = self.index;
        let first = self.parse_expr(0);
        if self.eat(TokenKind::Semicolon) {
            let (length, length_span) = self.parse_array_length("repeat count");
            let _ = self.expect(TokenKind::RBracket, "`]` to close the array literal");
            self.struct_literal_allowed = allowed;
            let span = start.cover(self.previous_span());
            return self.builder.push_expr(
                ExprKind::ArrayRepeat {
                    value: first,
                    length,
                    length_span,
                },
                span,
            );
        }

        let mut elements = vec![first];
        while self.eat(TokenKind::Comma) {
            if self.at(TokenKind::RBracket) || self.is_eof() {
                break;
            }
            elements.push(self.parse_expr(0));
        }
        if !self.at(TokenKind::RBracket) && !self.is_eof() && self.index == before {
            self.ensure_progress(before);
        }
        let _ = self.expect(TokenKind::RBracket, "`]` to close the array literal");
        self.struct_literal_allowed = allowed;
        let span = start.cover(self.previous_span());
        self.builder.push_expr(ExprKind::Array { elements }, span)
    }

    /// Parses `base[index]` or a range `base[start..end]`.
    fn parse_index(&mut self, base: ExprId) -> ExprId {
        self.bump();
        let allowed = self.struct_literal_allowed;
        self.struct_literal_allowed = true;
        let (start, end) = if self.eat(TokenKind::DotDot) {
            let end = if self.at(TokenKind::RBracket) {
                None
            } else {
                Some(self.parse_expr(0))
            };
            (None, end)
        } else {
            let first = self.parse_expr(0);
            if self.eat(TokenKind::DotDot) {
                let end = if self.at(TokenKind::RBracket) {
                    None
                } else {
                    Some(self.parse_expr(0))
                };
                (Some(first), end)
            } else {
                self.struct_literal_allowed = allowed;
                let _ = self.expect(TokenKind::RBracket, "`]` after the index");
                let span = self
                    .builder
                    .ast()
                    .expr(base)
                    .span
                    .cover(self.previous_span());
                return self
                    .builder
                    .push_expr(ExprKind::Index { base, index: first }, span);
            }
        };
        self.struct_literal_allowed = allowed;
        let _ = self.expect(TokenKind::RBracket, "`]` after the slice range");
        let span = self
            .builder
            .ast()
            .expr(base)
            .span
            .cover(self.previous_span());
        self.builder
            .push_expr(ExprKind::Slice { base, start, end }, span)
    }

    fn parse_call(&mut self, callee: ExprId) -> ExprId {
        let (args, end) = self.parse_call_arguments();
        let span = self.builder.ast().expr(callee).span.cover(end);
        self.builder
            .push_expr(ExprKind::Call { callee, args }, span)
    }

    /// Parses a parenthesized argument list, including both parentheses.
    fn parse_call_arguments(&mut self) -> (Vec<ExprId>, seal_span::Span) {
        self.bump();
        let allowed = self.struct_literal_allowed;
        self.struct_literal_allowed = true;
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
        self.struct_literal_allowed = allowed;
        let _ = self.expect(TokenKind::RParen, "`)` to close the call");
        (args, self.previous_span())
    }

    fn unary(&mut self, op: UnaryOp, operator: seal_span::Span, operand: ExprId) -> ExprId {
        let span = operator.cover(self.builder.ast().expr(operand).span);
        self.builder
            .push_expr(ExprKind::Unary { op, operand }, span)
    }

    pub(crate) fn suffix(&self, suffix: Option<NumericSuffix>) -> Option<Symbol> {
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
            ExprKind::Path(_)
            | ExprKind::Deref { .. }
            | ExprKind::Field { .. }
            | ExprKind::Index { .. } => true,
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
