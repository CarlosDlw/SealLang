//! Expressions: Pratt parsing with explicit binding powers.
//!
//! Precedence follows the C family, with assignment lowest and right-associative:
//! `||`, `&&`, `|`, `^`, `&`, equality, comparison, shifts, additive,
//! multiplicative, prefix, postfix/call. Assignment binds to a place and is
//! validated syntactically; whether the place is mutable is checked later.
//!
//! Prefix `&` and `*` are borrow and dereference, not the binary operators with
//! the same spelling: prefix position decides, so `&x & y` is `(&x) & y`.

use seal_ast::{
    AssignOp, BinaryOp, ExprId, ExprKind, FieldInit, Literal, PathSegment, PatternId, UnaryOp,
    UpdateOp,
};
use seal_error_codes as codes;
use seal_lexer::{NumericSuffix, TokenKind};
use seal_symbol::Symbol;

use crate::parser::{MAX_DEPTH, Parser, can_start_expr, is_expression_stop};

/// Binding power of assignment; lower than every binary operator.
const ASSIGN_BP: u8 = 1;
/// Binding power of prefix operators; higher than every binary operator.
const PREFIX_BP: u8 = 13;
/// Binding power of `as`; below prefix operators and above multiplicative ones.
const CAST_BP: u8 = 12;

/// The condition of one `if` or `if let` expression.
enum IfCondition {
    /// A boolean expression.
    Expr(ExprId),
    /// A refutable pattern and the value it is matched against.
    Let {
        pattern: PatternId,
        scrutinee: ExprId,
    },
}

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
                if let TokenKind::Int {
                    value,
                    base,
                    suffix,
                } = self.current_kind()
                {
                    let index_span = self.current_span();
                    if base != seal_lexer::NumericBase::Decimal || suffix.is_some() {
                        self.error(
                            codes::E1002,
                            index_span,
                            "a tuple index is a decimal integer without a suffix",
                        )
                        .help("write `.0`, `.1`, and so on")
                        .emit();
                    }
                    let index = u32::try_from(value).ok();
                    if index.is_none() {
                        self.error(codes::E1002, index_span, "this tuple index is too large")
                            .help("tuple indices are 32-bit unsigned integers")
                            .emit();
                    }
                    self.bump();
                    let span = self.builder.ast().expr(lhs).span.cover(index_span);
                    lhs = self.builder.push_expr(
                        ExprKind::TupleField {
                            base: lhs,
                            index: index.unwrap_or(0),
                            index_span,
                        },
                        span,
                    );
                    continue;
                }
                let (name, name_span) = self.expect_ident("a field name after `.`");
                let mut generic_args = None;
                if self.eat(TokenKind::ColonColon) {
                    if self.at(TokenKind::Lt) {
                        generic_args = self.parse_generic_args();
                    } else {
                        let found = self.describe(self.current_kind());
                        self.error(
                            codes::E1002,
                            self.current_span(),
                            format!("expected `<` after `::` on a method, found {found}"),
                        )
                        .help("write `.method::<T>()` for explicit method arguments")
                        .emit();
                    }
                } else if self.looks_like_bare_method_generics(name_span) {
                    generic_args = self.parse_generic_args();
                    let span = generic_args.as_ref().map_or(name_span, |args| args.span);
                    self.error(
                        codes::E1002,
                        span,
                        "type arguments on a method call require `::`",
                    )
                    .help("write `.method::<T>()` instead of `.method<T>()`")
                    .emit();
                }
                if self.at(TokenKind::LParen) {
                    let (args, end) = self.parse_call_arguments();
                    let span = self.builder.ast().expr(lhs).span.cover(end);
                    lhs = self.builder.push_expr(
                        ExprKind::MethodCall {
                            receiver: lhs,
                            name,
                            name_span,
                            generic_args,
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

            if self.at(TokenKind::As) {
                if min_bp > CAST_BP {
                    break;
                }
                let token = self.bump();
                let ty = self.parse_type();
                let span = self
                    .builder
                    .ast()
                    .expr(lhs)
                    .span
                    .cover(self.builder.ast().ty(ty).span);
                lhs = self.builder.push_expr(
                    ExprKind::Cast {
                        operand: lhs,
                        ty,
                        as_span: token.span,
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
            TokenKind::CStr(symbol) => {
                self.bump();
                self.builder
                    .push_expr(ExprKind::Lit(Literal::CStr(symbol)), token.span)
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
            TokenKind::Move => self.parse_closure(true, token.span),
            TokenKind::Pipe | TokenKind::OrOr => self.parse_closure(false, token.span),
            TokenKind::Ident(symbol) => {
                self.bump();
                self.parse_named_expr(symbol, token.span)
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
                if self.eat(TokenKind::RParen) {
                    self.struct_literal_allowed = allowed;
                    let span = token.span.cover(self.previous_span());
                    self.builder.push_expr(
                        ExprKind::Tuple {
                            elements: Vec::new(),
                        },
                        span,
                    )
                } else {
                    let first = self.parse_expr(0);
                    if self.eat(TokenKind::Comma) {
                        let mut elements = vec![first];
                        while !self.at(TokenKind::RParen) && !self.is_eof() {
                            let before = self.index;
                            elements.push(self.parse_expr(0));
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
                                format!("expected `,` or `)` in the tuple literal, found {found}"),
                            )
                            .help("separate tuple elements with `,`")
                            .emit();
                            if !can_start_expr(self.current_kind()) {
                                break;
                            }
                            self.ensure_progress(before);
                        }
                        let _ = self.expect(TokenKind::RParen, "`)` to close the tuple literal");
                        self.struct_literal_allowed = allowed;
                        let span = token.span.cover(self.previous_span());
                        self.builder.push_expr(ExprKind::Tuple { elements }, span)
                    } else {
                        let _ = self.expect(TokenKind::RParen, "`)`");
                        self.struct_literal_allowed = allowed;
                        let span = token.span.cover(self.previous_span());
                        self.builder.push_expr(ExprKind::Paren(first), span)
                    }
                }
            }
            TokenKind::LBracket => self.parse_array_literal(token.span),
            TokenKind::If => self.parse_if_expr(token.span),
            TokenKind::Match => self.parse_match_expr(token.span),
            TokenKind::Unsafe => {
                self.bump();
                let block = self.parse_block();
                let span = token.span.cover(self.previous_span());
                self.builder.push_unsafe(span, block)
            }
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

    /// Parses a closure expression starting at `start`.
    ///
    /// The caller consumed nothing, so the leading `move` (when `is_move`) and
    /// the parameter list are still at the cursor. Prefix `|` and `||` are
    /// closures; both stay binary operators everywhere else because only
    /// prefix position reaches this function.
    fn parse_closure(&mut self, is_move: bool, start: seal_span::Span) -> ExprId {
        if self.depth >= MAX_DEPTH {
            return self.depth_error_expr(start);
        }
        self.depth += 1;

        let mut params = Vec::new();
        if is_move {
            self.bump();
            if !self.at(TokenKind::Pipe) && !self.at(TokenKind::OrOr) {
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E1002,
                    self.current_span(),
                    format!("expected `|` after `move`, found {found}"),
                )
                .help("write `move |x| ...` or `move || ...`")
                .emit();
                self.depth -= 1;
                let span = start.cover(self.current_span());
                return self.builder.push_expr(ExprKind::Error, span);
            }
        }

        if self.eat(TokenKind::OrOr) {
            // `||` is the closure without parameters.
        } else {
            self.bump();
            while !self.at(TokenKind::Pipe) && !self.is_eof() {
                let before = self.index;
                let (name, name_span) = self.expect_ident("a closure parameter name");
                let ty = if self.eat(TokenKind::Colon) {
                    Some(self.parse_type())
                } else {
                    None
                };
                params.push(seal_ast::ClosureParam {
                    name,
                    name_span,
                    ty,
                });
                if self.eat(TokenKind::Comma) {
                    continue;
                }
                if self.at(TokenKind::Pipe) || self.is_eof() {
                    break;
                }
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E1002,
                    self.current_span(),
                    format!("expected `,` or `|` in the closure parameter list, found {found}"),
                )
                .help("separate closure parameters with `,`")
                .emit();
                if !can_start_expr(self.current_kind()) {
                    break;
                }
                self.ensure_progress(before);
            }
            if !self.eat(TokenKind::Pipe) {
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E1002,
                    self.current_span(),
                    format!("expected `|` to close the closure parameters, found {found}"),
                )
                .help("add the missing `|`")
                .emit();
            }
        }

        let ret = if self.eat(TokenKind::Arrow) {
            let ret = self.parse_type();
            if !self.at(TokenKind::LBrace) {
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E1002,
                    self.current_span(),
                    format!(
                        "a closure with an explicit result type needs a block body, found {found}"
                    ),
                )
                .help("write the body as `{ ... }`")
                .emit();
            }
            Some(ret)
        } else {
            None
        };

        let body = if self.at(TokenKind::LBrace) {
            let block = self.parse_block();
            let span = self.builder.ast().block(block).span;
            self.builder.push_expr(ExprKind::Block(block), span)
        } else {
            self.parse_expr(0)
        };

        self.depth -= 1;
        let span = start.cover(self.builder.ast().expr(body).span);
        self.builder.push_expr(
            ExprKind::Closure {
                is_move,
                params,
                ret,
                body,
            },
            span,
        )
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
        let condition = self.parse_if_condition();
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
        match condition {
            IfCondition::Expr(condition) => {
                self.builder
                    .push_if(span, condition, then_branch, else_branch)
            }
            IfCondition::Let { pattern, scrutinee } => {
                self.builder
                    .push_if_let(span, pattern, scrutinee, then_branch, else_branch)
            }
        }
    }

    /// Parses the condition of an `if` or `if let` expression.
    ///
    /// `let` is recognized only immediately after `if`, so it stays an ordinary
    /// keyword elsewhere. The scrutinee follows the same brace rule as a
    /// condition: a `{` starts the body, not a struct literal.
    fn parse_if_condition(&mut self) -> IfCondition {
        if !self.eat(TokenKind::Let) {
            return IfCondition::Expr(self.parse_condition());
        }

        let pattern = self.parse_match_pattern();
        if !self.eat(TokenKind::Eq) {
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1002,
                self.current_span(),
                format!("expected `=` after the `if let` pattern, found {found}"),
            )
            .help("write `if let pattern = value { ... }`")
            .emit();
        }

        let allowed = self.struct_literal_allowed;
        self.struct_literal_allowed = false;
        let scrutinee = self.parse_expr(0);
        self.struct_literal_allowed = allowed;
        IfCondition::Let { pattern, scrutinee }
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

    /// Parses a name, its optional turbofish, and any qualified continuation.
    ///
    /// One `::` keeps the existing `Type::associated` shape so every current
    /// program lowers exactly as before. Two or more segments become a path
    /// node for the module phase. A struct literal after a multi-segment path
    /// has its own node, because its type is not a single name.
    fn parse_named_expr(&mut self, name: Symbol, name_span: seal_span::Span) -> ExprId {
        if !self.eat(TokenKind::ColonColon) {
            return if self.struct_literal_allowed && self.at(TokenKind::LBrace) {
                self.parse_struct_literal(name, name_span, None)
            } else {
                self.path_expr(name, name_span, None)
            };
        }

        let type_args = self.parse_optional_generic_args();
        if type_args.is_some() && !self.at(TokenKind::ColonColon) {
            if self.struct_literal_allowed && self.at(TokenKind::LBrace) {
                return self.parse_struct_literal(name, name_span, type_args);
            }
            if self.at(TokenKind::Gt) || self.at(TokenKind::Shr) {
                self.error(
                    codes::E1002,
                    self.current_span(),
                    "unexpected `>` after the type arguments",
                )
                .help("remove the extra `>`")
                .emit();
                let _ = self.eat_gt();
            }
            return self.path_expr(name, name_span, type_args);
        }
        if type_args.is_some() {
            self.bump();
        }

        let (first, first_span) = self.expect_ident("an associated name after `::`");
        if first == Symbol::EMPTY {
            let span = name_span.cover(first_span);
            return self.builder.push_expr(
                ExprKind::PathQualified {
                    type_name: name,
                    type_span: name_span,
                    type_args,
                    name: first,
                    name_span: first_span,
                },
                span,
            );
        }
        let first_args = self.parse_segment_args();
        let mut segments = vec![PathSegment {
            name: first,
            name_span: first_span,
            args: first_args,
        }];
        while self.eat(TokenKind::ColonColon) {
            let (segment, segment_span) = self.expect_ident("a path segment after `::`");
            if segment == Symbol::EMPTY {
                break;
            }
            let args = self.parse_segment_args();
            segments.push(PathSegment {
                name: segment,
                name_span: segment_span,
                args,
            });
        }

        // `root` is always a module, never a type, so `root::name` keeps the
        // module-path shape even with a single segment. Every other single
        // segment stays the associated path form, because the parser cannot
        // know whether it names a type or a module.
        if segments.len() == 1
            && segments[0].args.is_none()
            && self.interner.resolve(name) == "root"
        {
            let end = segments[0].name_span;
            let span = name_span.cover(end);
            if self.struct_literal_allowed && self.at(TokenKind::LBrace) {
                let fields = self.parse_field_initializers(InitializerContext::Struct);
                let span = span.cover(self.previous_span());
                return self.builder.push_expr(
                    ExprKind::StructMulti {
                        root: name,
                        root_span: name_span,
                        args: type_args,
                        segments,
                        fields,
                    },
                    span,
                );
            }
            return self.builder.push_expr(
                ExprKind::PathMulti {
                    root: name,
                    root_span: name_span,
                    args: type_args,
                    segments,
                },
                span,
            );
        }

        if segments.len() == 1 && segments[0].args.is_none() {
            let segment = &segments[0];
            if self.struct_literal_allowed && self.at(TokenKind::LBrace) {
                return self.parse_variant_literal(
                    name,
                    name_span,
                    type_args,
                    segment.name,
                    segment.name_span,
                );
            }
            if type_args.is_some() && (self.at(TokenKind::Gt) || self.at(TokenKind::Shr)) {
                self.error(
                    codes::E1002,
                    self.current_span(),
                    "unexpected `>` after the type arguments",
                )
                .help("remove the extra `>`")
                .emit();
                let _ = self.eat_gt();
            }
            let span = name_span.cover(segment.name_span);
            return self.builder.push_expr(
                ExprKind::PathQualified {
                    type_name: name,
                    type_span: name_span,
                    type_args,
                    name: segment.name,
                    name_span: segment.name_span,
                },
                span,
            );
        }

        let end = segments
            .last()
            .map_or(first_span, |segment| segment.name_span);
        let span = name_span.cover(end);
        if self.struct_literal_allowed && self.at(TokenKind::LBrace) {
            let fields = self.parse_field_initializers(InitializerContext::Struct);
            let span = span.cover(self.previous_span());
            return self.builder.push_expr(
                ExprKind::StructMulti {
                    root: name,
                    root_span: name_span,
                    args: type_args,
                    segments,
                    fields,
                },
                span,
            );
        }
        self.builder.push_expr(
            ExprKind::PathMulti {
                root: name,
                root_span: name_span,
                args: type_args,
                segments,
            },
            span,
        )
    }

    /// Parses `::<...>` when it applies to one path segment.
    fn parse_segment_args(&mut self) -> Option<seal_ast::GenericArgs> {
        if self.at(TokenKind::ColonColon) && matches!(self.peek_kind(), Some(TokenKind::Lt)) {
            self.bump();
            self.parse_generic_args()
        } else {
            None
        }
    }

    fn path_expr(
        &mut self,
        name: Symbol,
        name_span: seal_span::Span,
        args: Option<seal_ast::GenericArgs>,
    ) -> ExprId {
        let end = args.as_ref().map_or(name_span, |args| args.span);
        let span = name_span.cover(end);
        self.builder.push_expr(
            ExprKind::Path {
                name,
                name_span,
                args,
            },
            span,
        )
    }

    fn parse_struct_literal(
        &mut self,
        name: Symbol,
        start: seal_span::Span,
        type_args: Option<seal_ast::GenericArgs>,
    ) -> ExprId {
        let fields = self.parse_field_initializers(InitializerContext::Struct);
        let span = start.cover(self.previous_span());
        self.builder.push_expr(
            ExprKind::Struct {
                name,
                name_span: start,
                type_args,
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
        type_args: Option<seal_ast::GenericArgs>,
        name: Symbol,
        name_span: seal_span::Span,
    ) -> ExprId {
        let fields = self.parse_field_initializers(InitializerContext::Variant);
        let span = type_span.cover(self.previous_span());
        self.builder.push_expr(
            ExprKind::Variant {
                type_name,
                type_span,
                type_args,
                name,
                name_span,
                fields,
            },
            span,
        )
    }

    /// Returns `true` for `value.method<Type>()`: generic arguments written
    /// without the required turbofish `::`.
    fn looks_like_bare_method_generics(&self, name_span: seal_span::Span) -> bool {
        if !self.at(TokenKind::Lt) || name_span.hi() != self.current_span().lo() {
            return false;
        }
        let mut depth = 0i32;
        for token in &self.tokens[self.index..] {
            match token.kind {
                TokenKind::Lt => depth += 1,
                TokenKind::Gt => depth -= 1,
                TokenKind::Shr => depth -= 2,
                TokenKind::LParen if depth <= 0 => return true,
                TokenKind::Semicolon
                | TokenKind::LBrace
                | TokenKind::RBrace
                | TokenKind::Eq
                | TokenKind::Eof => return false,
                _ => {}
            }
        }
        false
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
            ExprKind::Path { .. }
            | ExprKind::Deref { .. }
            | ExprKind::Field { .. }
            | ExprKind::TupleField { .. }
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
