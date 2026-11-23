//! Generic parameter lists, type arguments, bounds, and where clauses.

use seal_ast::{
    BoundKind, GenericArgs, GenericParams, TraitPath, TypeKind, TypePath, WhereClause,
    WherePredicate,
};
use seal_error_codes as codes;
use seal_lexer::{Token, TokenKind};
use seal_span::Span;
use seal_symbol::Symbol;

use crate::parser::Parser;
use crate::types::can_start_type;

impl Parser<'_> {
    /// Parses a generic parameter list when one follows, anchored at the current
    /// token when absent.
    pub(crate) fn parse_generic_params(&mut self) -> GenericParams {
        let start = self.current_span();
        if !self.eat(TokenKind::Lt) {
            return GenericParams::empty(start);
        }

        if self.at_generic_close() {
            let _ = self.eat_gt();
            let span = start.cover(self.previous_span());
            self.error(
                codes::E1002,
                span,
                "a generic parameter list cannot be empty",
            )
            .help("write at least one parameter, such as `<T>`")
            .emit();
            return GenericParams {
                params: Vec::new(),
                span,
            };
        }

        let mut params = Vec::new();
        loop {
            if self.at_generic_close() || self.is_eof() {
                break;
            }
            let before = self.index;
            let (name, name_span) = self.expect_ident("a generic parameter name");
            let bounds = if self.eat(TokenKind::Colon) {
                self.parse_bounds()
            } else {
                Vec::new()
            };
            let span = name_span.cover(self.previous_span());
            params.push(
                self.builder
                    .push_generic_param(name, name_span, bounds, span),
            );

            if self.eat(TokenKind::Comma) {
                if self.at_generic_close() || self.is_eof() {
                    break;
                }
                continue;
            }
            if self.at_generic_close() || self.is_eof() {
                break;
            }
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1002,
                self.current_span(),
                format!("expected `,` or `>` in the generic parameter list, found {found}"),
            )
            .help("separate generic parameters with `,`")
            .emit();
            if !matches!(self.current_kind(), TokenKind::Ident(_)) {
                break;
            }
            self.ensure_progress(before);
        }

        let close = if self.eat_gt() {
            self.previous_span()
        } else {
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1002,
                self.current_span(),
                format!("expected `>` to close the generic parameter list, found {found}"),
            )
            .help("add the missing `>`")
            .emit();
            self.current_span()
        };
        let span = start.cover(close);
        GenericParams { params, span }
    }

    /// Parses an angle-bracketed type argument list when one follows.
    pub(crate) fn parse_optional_generic_args(&mut self) -> Option<GenericArgs> {
        if self.at(TokenKind::Lt) {
            self.parse_generic_args()
        } else {
            None
        }
    }

    /// Parses `<Type, ...>` and returns the argument list.
    ///
    /// An empty list is diagnosed but still represented, so callers can keep
    /// lowering through the rest of the item.
    pub(crate) fn parse_generic_args(&mut self) -> Option<GenericArgs> {
        if !self.eat(TokenKind::Lt) {
            return None;
        }
        let start = self.previous_span();

        if self.at_generic_close() {
            let _ = self.eat_gt();
            let span = start.cover(self.previous_span());
            self.error(
                codes::E1002,
                span,
                "a generic argument list cannot be empty",
            )
            .help("remove the `<>`, or supply at least one type argument")
            .emit();
            return Some(GenericArgs {
                args: Vec::new(),
                span,
            });
        }

        let mut args = Vec::new();
        let mut reported_separator = false;
        loop {
            if self.at_generic_close() || self.is_eof() {
                break;
            }
            let before = self.index;
            args.push(self.parse_type());

            if self.eat(TokenKind::Comma) {
                if self.at_generic_close() || self.is_eof() {
                    break;
                }
                continue;
            }
            if self.at_generic_close() || self.is_eof() {
                break;
            }
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1002,
                self.current_span(),
                format!("expected `,` or `>` in the type argument list, found {found}"),
            )
            .help("separate type arguments with `,`")
            .emit();
            reported_separator = true;
            if !can_start_type(self.current_kind()) {
                break;
            }
            self.ensure_progress(before);
        }

        let close = if self.eat_gt() {
            self.previous_span()
        } else {
            // A closing delimiter already produced the targeted "expected `,`
            // or `>`" message; repeating it as a missing `>` only adds noise.
            let already_explained = reported_separator
                && matches!(self.current_kind(), TokenKind::RParen | TokenKind::RBracket);
            if !already_explained {
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E1002,
                    self.current_span(),
                    format!("expected `>` to close the type argument list, found {found}"),
                )
                .help("add the missing `>`")
                .emit();
            }
            self.current_span()
        };
        let span = start.cover(close);
        Some(GenericArgs { args, span })
    }

    fn parse_bounds(&mut self) -> Vec<seal_ast::BoundId> {
        let mut bounds = Vec::new();
        loop {
            if self.at_bounds_end() || self.is_eof() {
                if bounds.is_empty() {
                    self.error(
                        codes::E1002,
                        self.current_span(),
                        "expected a bound after `:`",
                    )
                    .help("name a trait, or write the built-in `Copy` bound")
                    .emit();
                }
                break;
            }
            if !matches!(self.current_kind(), TokenKind::Ident(_)) {
                self.error(codes::E1002, self.current_span(), "expected a bound name")
                    .help("name a trait, or write the built-in `Copy` bound")
                    .emit();
                if self.eat(TokenKind::Plus) {
                    continue;
                }
                break;
            }

            let (name, name_span) = self.expect_ident("a bound name");
            let args = if self.is_fn_trait_name(name) && self.at(TokenKind::LParen) {
                Some(self.parse_fn_trait_sugar(name_span))
            } else {
                self.parse_optional_generic_args()
            };
            let end = args.as_ref().map_or(name_span, |args| args.span);
            let span = name_span.cover(end);
            let kind = if self.is_copy_bound(name, args.is_some()) {
                BoundKind::Copy
            } else {
                BoundKind::Trait(TraitPath {
                    name,
                    name_span,
                    args,
                    span,
                })
            };
            bounds.push(self.builder.push_bound(kind, span));

            if self.eat(TokenKind::Plus) {
                if self.at_bounds_end() {
                    self.error(
                        codes::E1002,
                        self.current_span(),
                        "expected a bound after `+`",
                    )
                    .help("remove the trailing `+`, or name another bound")
                    .emit();
                    break;
                }
                continue;
            }
            break;
        }
        bounds
    }

    /// Returns `true` when `name` spells one of the built-in function traits.
    pub(crate) fn is_fn_trait_name(&self, name: Symbol) -> bool {
        matches!(self.interner.resolve(name), "Fn" | "FnMut" | "FnOnce")
    }

    /// Parses the `(A, B) -> R` sugar of a function-trait bound.
    ///
    /// The sugar is represented as the explicit form `Fn<(A, B), R>`: a
    /// synthetic tuple type plus the result type, with `()` when the source
    /// omits the arrow. One representation means name resolution, bounds
    /// validation, and later phases never branch on which spelling was used.
    ///
    /// The caller has already read the trait name and is at `(`.
    pub(crate) fn parse_fn_trait_sugar(&mut self, name_span: Span) -> GenericArgs {
        let open = self.current_span();
        self.bump();

        let mut params = Vec::new();
        let close = if self.eat(TokenKind::RParen) {
            self.previous_span()
        } else {
            loop {
                let before = self.index;
                params.push(self.parse_type());
                if self.eat(TokenKind::Comma) {
                    if self.at(TokenKind::RParen) || self.is_eof() {
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
                    format!("expected `,` or `)` in the function parameter list, found {found}"),
                )
                .help("separate parameter types with `,`")
                .emit();
                if !can_start_type(self.current_kind()) {
                    break;
                }
                self.ensure_progress(before);
            }
            if self.eat(TokenKind::RParen) {
                self.previous_span()
            } else {
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E1002,
                    self.current_span(),
                    format!("expected `)` to close the function parameter list, found {found}"),
                )
                .help("add the missing `)`")
                .emit();
                self.current_span()
            }
        };

        let tuple = self
            .builder
            .push_type(TypeKind::Tuple(params), open.cover(close));
        let output = if self.eat(TokenKind::Arrow) {
            self.parse_type()
        } else {
            self.builder.push_type(TypeKind::Tuple(Vec::new()), close)
        };
        let span = name_span.cover(self.builder.ast().ty(output).span);
        GenericArgs {
            args: vec![tuple, output],
            span,
        }
    }

    /// Parses a trailing `where` clause, or returns an empty clause anchored at
    /// the current token.
    pub(crate) fn parse_where_clause(&mut self) -> WhereClause {
        let start = self.current_span();
        if !self.eat(TokenKind::Where) {
            return WhereClause::empty(start);
        }

        if self.at_where_end() || self.is_eof() {
            self.error(
                codes::E1002,
                start.cover(self.previous_span()),
                "a where clause needs at least one predicate",
            )
            .help("write `where Type: Bound`, or remove `where`")
            .emit();
            return WhereClause::empty(start.cover(self.previous_span()));
        }

        let mut predicates = Vec::new();
        while !self.at_where_end() && !self.is_eof() {
            let before = self.index;
            let ty = self.parse_type();
            if !self.eat(TokenKind::Colon) {
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E1002,
                    self.current_span(),
                    format!("expected `:` after the where-clause type, found {found}"),
                )
                .help("write `where Type: Bound`")
                .emit();
            }
            let bounds = self.parse_bounds();
            let end = bounds
                .last()
                .map_or(self.builder.ast().ty(ty).span, |bound| {
                    self.builder.ast().bound(*bound).span
                });
            let span = self.builder.ast().ty(ty).span.cover(end);
            predicates.push(WherePredicate { ty, bounds, span });

            if self.eat(TokenKind::Comma) {
                if self.at_where_end() || self.is_eof() {
                    break;
                }
                continue;
            }
            if self.at_where_end() || self.is_eof() {
                break;
            }
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1002,
                self.current_span(),
                format!("expected `,` between where predicates, found {found}"),
            )
            .help("separate predicates with `,`")
            .emit();
            if !can_start_type(self.current_kind()) {
                break;
            }
            self.ensure_progress(before);
        }

        let span = start.cover(self.previous_span());
        WhereClause { predicates, span }
    }

    /// Returns `true` when the current identifier starts a trait path followed
    /// by `for`, so the impl parser can avoid building an orphan type node.
    pub(crate) fn starts_trait_path_before_for(&self) -> bool {
        if !matches!(self.current_kind(), TokenKind::Ident(_)) {
            return false;
        }
        let mut depth = 0i32;
        let mut index = self.index + 1;
        while let Some(token) = self.tokens.get(index) {
            match token.kind {
                TokenKind::Lt => depth += 1,
                TokenKind::Gt => depth -= 1,
                TokenKind::Shr => depth -= 2,
                TokenKind::For if depth <= 0 => return true,
                TokenKind::Comma | TokenKind::Semicolon | TokenKind::LBrace | TokenKind::Eof => {
                    return false;
                }
                _ => {}
            }
            index += 1;
        }
        false
    }

    /// Parses a trait path without creating a throwaway type node.
    pub(crate) fn parse_trait_path(&mut self) -> TraitPath {
        let (name, name_span) = self.expect_ident("a trait name");
        let args = self.parse_optional_generic_args();
        let end = args.as_ref().map_or(name_span, |args| args.span);
        TraitPath {
            name,
            name_span,
            args,
            span: name_span.cover(end),
        }
    }

    /// Converts the first path of an impl header into a trait path.
    ///
    /// The parser does not know whether the first name names a trait, so it
    /// parses a type and preserves its arguments; HIR reports a name that is not
    /// a trait.
    pub(crate) fn trait_path_from_type(&mut self, ty: seal_ast::TypeId) -> TraitPath {
        let span = self.builder.ast().ty(ty).span;
        match self.builder.ast().ty(ty).kind.clone() {
            TypeKind::Path(TypePath {
                name,
                name_span,
                args,
            }) => TraitPath {
                name,
                name_span,
                args,
                span,
            },
            TypeKind::Primitive(_) => {
                self.error(codes::E1002, span, "expected a trait name before `for`")
                    .help("write `impl TraitName for TypeName`")
                    .emit();
                TraitPath::new(Symbol::EMPTY, span)
            }
            _ => {
                self.error(codes::E1002, span, "expected a trait name before `for`")
                    .help("write `impl TraitName for TypeName`")
                    .emit();
                TraitPath::new(Symbol::EMPTY, span)
            }
        }
    }

    fn is_copy_bound(&self, name: Symbol, has_args: bool) -> bool {
        !has_args && self.interner.resolve(name) == "Copy"
    }

    fn at_bounds_end(&self) -> bool {
        self.at(TokenKind::Comma)
            || self.at(TokenKind::Semicolon)
            || self.at(TokenKind::LBrace)
            || self.at(TokenKind::RBrace)
            || self.at(TokenKind::RParen)
            || self.at_generic_close()
    }

    fn at_where_end(&self) -> bool {
        self.at(TokenKind::LBrace)
            || self.at(TokenKind::Semicolon)
            || self.at(TokenKind::RBrace)
            || self.at(TokenKind::Eq)
    }

    fn at_generic_close(&self) -> bool {
        self.at(TokenKind::Gt) || self.at(TokenKind::Shr)
    }

    /// Consumes one `>`; splits `>>` into two virtual tokens.
    pub(crate) fn eat_gt(&mut self) -> bool {
        if self.at(TokenKind::Gt) {
            self.bump();
            return true;
        }
        if self.at(TokenKind::Shr) {
            let token = self.current();
            let (first, second) = token.span.split(1);
            self.index += 1;
            self.previous = first;
            self.pending = Some(Token {
                kind: TokenKind::Gt,
                span: second,
                preceded_by_newline: false,
            });
            return true;
        }
        false
    }
}
