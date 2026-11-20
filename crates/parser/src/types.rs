//! Type syntax.

use seal_ast::{PathSegment, PrimitiveType, TraitPath, TypeId, TypeKind, TypePath};
use seal_error_codes as codes;
use seal_lexer::TokenKind;
use seal_symbol::Symbol;

use crate::parser::{MAX_DEPTH, Parser};

impl Parser<'_> {
    /// Parses a type: a primitive, reference, tuple, array, slice, named path,
    /// or trait object such as `dyn Shape`.
    pub(crate) fn parse_type(&mut self) -> TypeId {
        let start = self.current_span();
        if self.depth >= MAX_DEPTH {
            return self.depth_error_type(start);
        }
        self.depth += 1;

        let result = if self.eat(TokenKind::Star) {
            let raw = self.at_contextual("raw");
            if raw {
                self.bump();
            }
            let mutable = self.eat(TokenKind::Mut);
            if raw && self.at_contextual("extern") {
                // A C function pointer is always read-only: the pointer value
                // names code, not mutable storage. Reject `mut` and recover as
                // the read-only form so one typo does not cascade.
                if mutable {
                    self.error(
                        codes::E3089,
                        start.cover(self.current_span()),
                        "a C function pointer cannot be `mut`",
                    )
                    .help("remove `mut`; function pointers are read-only")
                    .emit();
                }
                let pointee = self.parse_fn_sig_type();
                let span = start.cover(self.builder.ast().ty(pointee).span);
                self.builder.push_type(
                    TypeKind::RawPointer {
                        pointee,
                        mutable: false,
                    },
                    span,
                )
            } else {
                let pointee = self.parse_type();
                let span = start.cover(self.builder.ast().ty(pointee).span);
                let kind = if raw {
                    TypeKind::RawPointer { pointee, mutable }
                } else {
                    TypeKind::Pointer { pointee, mutable }
                };
                self.builder.push_type(kind, span)
            }
        } else if self.eat(TokenKind::LParen) {
            if self.eat(TokenKind::RParen) {
                let span = start.cover(self.previous_span());
                self.builder.push_type(TypeKind::Tuple(Vec::new()), span)
            } else {
                let first = self.parse_type();
                if self.eat(TokenKind::Comma) {
                    let mut elements = vec![first];
                    while !self.at(TokenKind::RParen) && !self.is_eof() {
                        let before = self.index;
                        elements.push(self.parse_type());
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
                            format!("expected `,` or `)` in the tuple type, found {found}"),
                        )
                        .help("separate tuple element types with `,`")
                        .emit();
                        if !can_start_type(self.current_kind()) {
                            break;
                        }
                        self.ensure_progress(before);
                    }
                    let _ = self.expect(TokenKind::RParen, "`)` to close the tuple type");
                    let span = start.cover(self.previous_span());
                    self.builder.push_type(TypeKind::Tuple(elements), span)
                } else {
                    // `(T)` is a parenthesized type, not a one-element tuple.
                    let _ = self.expect(TokenKind::RParen, "`)` to close the type");
                    first
                }
            }
        } else if self.eat(TokenKind::LBracket) {
            let element = self.parse_type();
            if self.eat(TokenKind::Semicolon) {
                let (length, length_span) = self.parse_array_length("array length");
                let _ = self.expect(TokenKind::RBracket, "`]` to close the array type");
                let span = start.cover(self.previous_span());
                self.builder.push_type(
                    TypeKind::Array {
                        element,
                        length,
                        length_span,
                    },
                    span,
                )
            } else {
                let _ = self.expect(TokenKind::RBracket, "`]` to close the slice type");
                let span = start.cover(self.previous_span());
                self.builder.push_type(TypeKind::Slice { element }, span)
            }
        } else if self.eat(TokenKind::Dyn) {
            let trait_path = self.parse_dyn_trait_path();
            let span = start.cover(self.previous_span());
            self.builder.push_type(TypeKind::Dyn(trait_path), span)
        } else if self.at(TokenKind::Unsafe) && matches!(self.peek_kind(), Some(TokenKind::Func)) {
            self.parse_seal_fn_ptr_type(true)
        } else if self.at(TokenKind::Func) {
            self.parse_seal_fn_ptr_type(false)
        } else if self.at_contextual("extern")
            && matches!(self.peek_kind(), Some(TokenKind::Str(_)))
        {
            // A C signature is never a value type. Recovering as the C
            // signature keeps the rest of the declaration checkable, and the
            // diagnostic points at the raw-pointer spelling that is valid.
            self.error(
                codes::E3099,
                start,
                "a C function signature is not a Seal value type",
            )
            .help(
                "write `*raw extern \"c\" func(...)` for a C function pointer, or `func(...)` for a Seal one",
            )
            .emit();
            self.parse_fn_sig_type()
        } else if let TokenKind::Ident(symbol) = self.current_kind() {
            let text = self.interner.resolve(symbol);
            match PrimitiveType::from_name(text) {
                Some(primitive) => {
                    let token = self.bump();
                    self.builder
                        .push_type(TypeKind::Primitive(primitive), token.span)
                }
                None => {
                    // Named types are resolved during lowering, where the type
                    // namespace is known; the parser records the name and any
                    // explicit arguments. Two or more `::` segments keep every
                    // name for module resolution in the next phase.
                    let token = self.bump();
                    let args = self.parse_type_segment_args();
                    let mut segments = Vec::new();
                    let mut end = args.as_ref().map_or(token.span, |args| args.span);
                    while self.eat(TokenKind::ColonColon) {
                        let (name, name_span) = self.expect_ident("a type path segment");
                        if name == Symbol::EMPTY {
                            break;
                        }
                        let args = self.parse_type_segment_args();
                        end = name_span;
                        if let Some(args) = &args {
                            end = args.span;
                        }
                        segments.push(PathSegment {
                            name,
                            name_span,
                            args,
                        });
                    }
                    if segments.is_empty() {
                        let path = TypePath {
                            name: symbol,
                            name_span: token.span,
                            args,
                        };
                        self.builder
                            .push_type(TypeKind::Path(path), token.span.cover(end))
                    } else {
                        self.builder.push_type(
                            TypeKind::PathMulti {
                                root: symbol,
                                root_span: token.span,
                                args,
                                segments,
                            },
                            token.span.cover(end),
                        )
                    }
                }
            }
        } else {
            let span = self.current_span();
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1004,
                span,
                format!("expected a type, found {found}"),
            )
            .help("write a built-in type name, or a reference such as `*i32` or `*mut i32`")
            .emit();
            if !is_type_stop(self.current_kind()) {
                self.bump();
            }
            self.builder.push_type(TypeKind::Error, span)
        };

        self.depth -= 1;
        result
    }

    /// Parses `func(T, ...) -> R` and `unsafe func(T, ...) -> R`.
    ///
    /// The caller leaves `unsafe` (when present) and `func` at the cursor. The
    /// signature is structural: the same parameter and result types produce the
    /// same function pointer type, and the list is never variadic.
    fn parse_seal_fn_ptr_type(&mut self, is_unsafe: bool) -> TypeId {
        let start = self.current_span();
        if is_unsafe {
            self.bump();
        }
        self.bump();

        let mut params = Vec::new();
        if self.eat(TokenKind::LParen) {
            while !self.at(TokenKind::RParen) && !self.is_eof() {
                let before = self.index;
                if self.at(TokenKind::DotDotDot) {
                    let span = self.current_span();
                    self.error(
                        codes::E3099,
                        span,
                        "a Seal function type cannot be variadic",
                    )
                    .help("remove `...`; Seal function pointers take a fixed parameter list")
                    .emit();
                    while !self.at(TokenKind::RParen) && !self.is_eof() {
                        self.bump();
                    }
                    break;
                }
                params.push(self.parse_type());
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
                    format!("expected `,` between function parameter types, found {found}"),
                )
                .help("separate parameter types with `,`")
                .emit();
                if !can_start_type(self.current_kind()) {
                    break;
                }
                self.ensure_progress(before);
            }
            let _ = self.expect(
                TokenKind::RParen,
                "`)` to close the function parameter list",
            );
        } else {
            self.error(
                codes::E1002,
                self.current_span(),
                "expected `(` after `func`",
            )
            .help("write the parameter types, as in `func(i32) -> i32`")
            .emit();
        }

        let ret = self.eat(TokenKind::Arrow).then(|| self.parse_type());
        let span = start.cover(self.previous_span());
        self.builder.push_type(
            TypeKind::FnPtr {
                params,
                ret,
                is_unsafe,
            },
            span,
        )
    }

    /// Parses `extern "c" func(T, ...) -> R` after `*raw`.
    ///
    /// The caller has already consumed `*raw`. The signature is a type-level
    /// description of a C function, so its parameter list contains types only;
    /// names would be a declaration detail, not part of the pointer type.
    fn parse_fn_sig_type(&mut self) -> TypeId {
        let start = self.current_span();
        self.bump();
        let (abi, abi_span) = self.parse_abi();
        if !self.eat(TokenKind::Func) {
            let found = self.describe(self.current_kind());
            self.error(
                codes::E3081,
                self.current_span(),
                format!("expected `func` after the C function ABI, found {found}"),
            )
            .help("write the parameter list after `func`, as in `*raw extern \"c\" func()`")
            .emit();
        }

        let mut params = Vec::new();
        let mut variadic = false;
        let mut variadic_span = None;
        if self.eat(TokenKind::LParen) {
            while !self.at(TokenKind::RParen) && !self.is_eof() {
                let before = self.index;
                if self.at(TokenKind::DotDotDot) {
                    variadic_span = Some(self.current_span());
                    self.bump();
                    variadic = true;
                    break;
                }
                params.push(self.parse_type());
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
                    format!("expected `,` between C function parameters, found {found}"),
                )
                .help("separate parameter types with `,`")
                .emit();
                if !can_start_type(self.current_kind()) {
                    break;
                }
                self.ensure_progress(before);
            }
            if variadic && !self.at(TokenKind::RParen) {
                self.error(
                    codes::E3088,
                    self.current_span(),
                    "`...` must be the last element of the parameter list",
                )
                .help("move `...` to the end, after every fixed parameter")
                .emit();
                while !self.at(TokenKind::RParen) && !self.is_eof() {
                    self.bump();
                }
            }
            if let Some(span) = variadic_span
                && params.is_empty()
            {
                self.error(
                    codes::E3088,
                    span,
                    "a variadic C function needs at least one fixed parameter",
                )
                .help("declare the fixed parameters C always receives, then `...`")
                .emit();
            }
            let _ = self.expect(TokenKind::RParen, "`)` after the parameter list");
        } else if !self.is_eof() {
            self.error(
                codes::E1002,
                self.current_span(),
                "expected `(` after `func`",
            )
            .help("write the parameter types, as in `func(i32) -> i32`")
            .emit();
        }

        let ret = self.eat(TokenKind::Arrow).then(|| self.parse_type());
        let span = start.cover(self.previous_span());
        self.builder.push_type(
            TypeKind::FnSig {
                abi,
                abi_span,
                params,
                ret,
                variadic,
                variadic_span,
            },
            span,
        )
    }

    /// Parses type arguments on one type path segment, with or without the
    /// expression-style `::` before `<`.
    fn parse_type_segment_args(&mut self) -> Option<seal_ast::GenericArgs> {
        if self.at(TokenKind::ColonColon) && matches!(self.peek_kind(), Some(TokenKind::Lt)) {
            self.bump();
            self.parse_generic_args()
        } else {
            self.parse_optional_generic_args()
        }
    }

    /// Returns `true` when the current token spells the contextual keyword.
    ///
    /// Contextual keywords are reserved in exactly one grammar position and
    /// remain ordinary identifiers elsewhere, so `raw` stays a valid name.
    fn at_contextual(&self, keyword: &str) -> bool {
        match self.current_kind() {
            TokenKind::Ident(symbol) => self.interner.resolve(symbol) == keyword,
            _ => false,
        }
    }

    /// Parses the trait path after `dyn` and recovers without cascading into
    /// the surrounding type or expression parser.
    fn parse_dyn_trait_path(&mut self) -> TraitPath {
        if !matches!(self.current_kind(), TokenKind::Ident(_)) {
            let span = self.current_span();
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1002,
                span,
                format!("expected a trait name after `dyn`, found {found}"),
            )
            .help("write the trait the object dispatches to, as in `dyn Shape`")
            .emit();
            self.skip_invalid_dyn_tail();
            return TraitPath::new(Symbol::EMPTY, span);
        }

        let mut path = self.parse_trait_path();
        if path.args.is_none() && self.is_fn_trait_name(path.name) && self.at(TokenKind::LParen) {
            // Accept the bound sugar so `dyn Fn(...)` reaches HIR, which
            // rejects function-trait objects with a targeted diagnostic.
            let sugar = self.parse_fn_trait_sugar(path.name_span);
            path.span = path.span.cover(sugar.span);
            path.args = Some(sugar);
        }
        if self.at(TokenKind::Plus) {
            let span = self.current_span();
            self.error(codes::E1002, span, "a `dyn` type takes exactly one trait")
                .help("remove the extra trait; Seal has no trait-object composition")
                .emit();
            self.skip_invalid_dyn_tail();
        }
        path
    }

    /// Consumes an invalid `dyn` tail up to a delimiter that the surrounding
    /// grammar still needs. Angle brackets are tracked so a malformed
    /// `Other<T>` does not stop at its own `>`.
    fn skip_invalid_dyn_tail(&mut self) {
        let mut depth = 0i32;
        while !self.is_eof() {
            match self.current_kind() {
                TokenKind::Lt => depth += 1,
                TokenKind::Gt if depth > 0 => depth -= 1,
                TokenKind::Shr if depth > 0 => depth -= 2,
                TokenKind::Gt | TokenKind::Shr if depth <= 0 => break,
                TokenKind::Comma
                | TokenKind::Semicolon
                | TokenKind::Colon
                | TokenKind::Eq
                | TokenKind::Arrow
                | TokenKind::RParen
                | TokenKind::RBracket
                | TokenKind::LBrace
                | TokenKind::RBrace
                    if depth <= 0 =>
                {
                    break;
                }
                _ => {}
            }
            self.bump();
        }
    }
}

/// Delimiters after which a missing type should not force token consumption:
/// the initializer or the surrounding syntax still needs them.
pub(crate) fn can_start_type(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Star
            | TokenKind::LBracket
            | TokenKind::LParen
            | TokenKind::Dyn
            | TokenKind::Func
            | TokenKind::Unsafe
            | TokenKind::Ident(_)
    )
}

fn is_type_stop(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Eof
            | TokenKind::LBrace
            | TokenKind::RBrace
            | TokenKind::RParen
            | TokenKind::RBracket
            | TokenKind::Comma
            | TokenKind::Semicolon
            | TokenKind::Eq
            | TokenKind::Arrow
            | TokenKind::Colon
            | TokenKind::Plus
            | TokenKind::Gt
            | TokenKind::Shr
            | TokenKind::Pipe
    )
}
