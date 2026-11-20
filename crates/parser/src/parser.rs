//! Parser state, token access, diagnostics, and top-level items.

use seal_ast::{
    Ast, AstBuilder, ConstDecl, EnumDecl, ExprId, ExprKind, FieldDeclId, FnDecl, GenericParams,
    ImplDecl, ItemId, ItemKind, ParamId, StaticDecl, TraitDecl, TraitMethodDecl, TupleFieldDeclId,
    TypeId, TypeKind, VariantDeclId, VariantFieldList,
};
use seal_diagnostics::{DiagCtxt, ErrorBuilder};
use seal_error_codes as codes;
use seal_error_codes::ErrorCode;
use seal_lexer::{Lexed, Token, TokenKind};
use seal_span::Span;
use seal_symbol::{Interner, Symbol};

/// The name, type, and initializer of one `const` or `static` declaration.
pub(crate) struct ValueDecl {
    pub(crate) name: Symbol,
    pub(crate) name_span: Span,
    pub(crate) ty: TypeId,
    pub(crate) init: ExprId,
}

/// Maximum nesting depth for expressions and types.
///
/// The limit protects the process stack. Valid programs stay far below it; it
/// only matters for generated code or hostile input.
pub(crate) const MAX_DEPTH: usize = 256;

/// One parsing pass over a token stream.
pub(crate) struct Parser<'a> {
    pub(crate) tokens: &'a [Token],
    pub(crate) index: usize,
    /// A virtual token produced by splitting a multi-character token. The lexer
    /// emits `>>`, but a nested generic list needs two `>` tokens in source
    /// order. Nothing else uses virtual tokens.
    pub(crate) pending: Option<Token>,
    pub(crate) previous: Span,
    pub(crate) dcx: &'a DiagCtxt,
    pub(crate) interner: &'a Interner,
    pub(crate) builder: AstBuilder,
    pub(crate) items: Vec<ItemId>,
    pub(crate) depth: usize,
    /// Whether a `{` after a name starts a struct literal. Conditions disable
    /// it so the condition of `if`/`while` cannot swallow the body.
    pub(crate) struct_literal_allowed: bool,
}

impl<'a> Parser<'a> {
    pub(crate) fn new(lexed: &'a Lexed, dcx: &'a DiagCtxt, interner: &'a Interner) -> Self {
        let tokens = lexed.tokens.as_slice();
        debug_assert!(
            matches!(tokens.last().map(|token| token.kind), Some(TokenKind::Eof)),
            "parser requires a lexed token stream ending with Eof"
        );
        let previous = tokens.first().map_or(Span::default(), |token| token.span);
        Self {
            tokens,
            index: 0,
            pending: None,
            previous,
            dcx,
            interner,
            builder: AstBuilder::new(),
            items: Vec::new(),
            depth: 0,
            struct_literal_allowed: true,
        }
    }

    /// Parses every item and returns the finished tree.
    pub(crate) fn parse_program(mut self) -> Ast {
        while !self.is_eof() {
            let before = self.index;
            if self.at(TokenKind::Func) {
                let start = self.current_span();
                let item = self.parse_function(false, false, start);
                self.items.push(item);
            } else if self.at(TokenKind::Unsafe) {
                let start = self.current_span();
                let item = self.parse_unsafe_item(start);
                self.items.push(item);
            } else if self.at(TokenKind::Const) {
                let item = self.parse_const_item();
                self.items.push(item);
            } else if self.at(TokenKind::Static) {
                let item = self.parse_static_item();
                self.items.push(item);
            } else if self.at(TokenKind::Struct) {
                let item = self.parse_struct();
                self.items.push(item);
            } else if self.at(TokenKind::Enum) {
                let item = self.parse_enum();
                self.items.push(item);
            } else if self.at(TokenKind::Impl) {
                let item = self.parse_impl();
                self.items.push(item);
            } else if self.at(TokenKind::Trait) {
                let item = self.parse_trait();
                self.items.push(item);
            } else if self.at_name("extern") {
                let item = self.parse_extern_item();
                self.items.push(item);
            } else {
                let start = self.current_span();
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E1001,
                    start,
                    format!("expected an item, found {found}"),
                )
                .help(
                    "top-level items start with `func`, `const`, `static`, `struct`, `enum`, \
                     `impl`, or `trait`",
                )
                .emit();
                while !self.is_eof()
                    && !self.at(TokenKind::Func)
                    && !self.at(TokenKind::Unsafe)
                    && !self.at(TokenKind::Const)
                    && !self.at(TokenKind::Static)
                    && !self.at(TokenKind::Struct)
                    && !self.at(TokenKind::Enum)
                    && !self.at(TokenKind::Impl)
                    && !self.at(TokenKind::Trait)
                {
                    self.bump();
                }
                let span = start.cover(self.previous_span());
                let item = self.builder.push_item(ItemKind::Error, span);
                self.items.push(item);
            }
            self.ensure_progress(before);
        }

        let items = std::mem::take(&mut self.items);
        self.builder.finish(items)
    }

    /// Parses an item introduced by `unsafe`.
    ///
    /// `unsafe` is only meaningful on a function. A value or aggregate that
    /// follows it receives `E3071` and is still parsed, so one misplaced marker
    /// does not hide the rest of the file. Both `unsafe const func` and
    /// `const unsafe func` are accepted.
    fn parse_unsafe_item(&mut self, start: Span) -> ItemId {
        let keyword = self.bump();
        if self.at(TokenKind::Func) {
            return self.parse_function(false, true, start);
        }
        if self.at(TokenKind::Const) {
            self.bump();
            if self.at(TokenKind::Func) {
                return self.parse_function(true, true, start);
            }
            self.error(
                codes::E3071,
                keyword.span,
                "`unsafe` applies only to functions",
            )
            .help("remove `unsafe` from the constant declaration")
            .emit();
            return self.parse_const_after(start);
        }

        self.error(
            codes::E3071,
            keyword.span,
            "`unsafe` applies only to functions",
        )
        .help("remove `unsafe`; unsafe behavior belongs in an `unsafe func`")
        .emit();
        if self.at(TokenKind::Struct) {
            self.parse_struct()
        } else if self.at(TokenKind::Enum) {
            self.parse_enum()
        } else if self.at(TokenKind::Impl) {
            self.parse_impl()
        } else if self.at(TokenKind::Trait) {
            self.parse_trait()
        } else if self.at(TokenKind::Static) {
            self.parse_static_item()
        } else {
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1001,
                start,
                format!("expected an item after `unsafe`, found {found}"),
            )
            .help("write `unsafe func name(...) { ... }`")
            .emit();
            self.skip_declaration();
            let span = start.cover(self.previous_span());
            self.builder.push_item(ItemKind::Error, span)
        }
    }

    /// Parses a `const` item, including `const func`.
    ///
    /// The token after `const` decides the form: `func` starts a constant
    /// function, and anything else starts a constant declaration.
    fn parse_const_item(&mut self) -> ItemId {
        let keyword = self.bump();
        // `const unsafe func` uses `unsafe` as a marker. `const unsafe: T = v`
        // instead has a reserved keyword where the name belongs; leave that to
        // the value-declaration parser so it reports E1005 with its usual help.
        if self.at(TokenKind::Unsafe) && self.peek_kind() == Some(TokenKind::Func) {
            self.bump();
            return self.parse_function(true, true, keyword.span);
        }
        self.parse_const_after(keyword.span)
    }

    /// Parses the part of a `const` declaration after the keyword.
    ///
    /// `start` includes any preceding `unsafe` token so the item span covers
    /// the complete declaration.
    fn parse_const_after(&mut self, start: Span) -> ItemId {
        if self.at(TokenKind::Func) {
            return self.parse_function(true, false, start);
        }
        self.reject_const_mut();
        let declaration = self.parse_value_decl("const");
        let span = start.cover(self.previous_span());
        self.builder.push_const_item(
            span,
            ConstDecl {
                name: declaration.name,
                name_span: declaration.name_span,
                ty: declaration.ty,
                init: declaration.init,
            },
        )
    }

    /// Parses a `static` item, with an optional `mut`.
    fn parse_static_item(&mut self) -> ItemId {
        let keyword = self.bump();
        if self.at(TokenKind::Func) {
            self.error(
                codes::E1013,
                keyword.span.cover(self.current_span()),
                "`static` cannot declare a function",
            )
            .help("write `const func` for a function that may run at compile time")
            .emit();
            return self.parse_function(false, false, keyword.span);
        }
        let mutable = self.eat(TokenKind::Mut);
        self.reject_extra_mut();
        let declaration = self.parse_value_decl("static");
        let span = keyword.span.cover(self.previous_span());
        self.builder.push_static_item(
            span,
            StaticDecl {
                name: declaration.name,
                name_span: declaration.name_span,
                mutable,
                ty: declaration.ty,
                init: declaration.init,
            },
        )
    }

    /// Parses the name, type, and initializer shared by `const` and `static`
    /// declarations, at file scope and in a block.
    ///
    /// The current token is the name. A missing type or initializer is reported
    /// once as `E1012`, and the declaration still gets a total node with an
    /// error type or expression so later phases see the written shape.
    pub(crate) fn parse_value_decl(&mut self, keyword: &str) -> ValueDecl {
        let (name, name_span) = self.expect_ident("a declaration name");
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

        if ty.is_none() || init.is_none() {
            let message = match (ty.is_none(), init.is_none()) {
                (true, true) => {
                    format!("`{keyword}` declaration needs an explicit type and an initializer")
                }
                (true, false) => format!("`{keyword}` declaration needs an explicit type"),
                (false, true) => format!("`{keyword}` declaration needs an initializer"),
                (false, false) => unreachable!("a missing piece was just detected"),
            };
            self.error(codes::E1012, name_span, message)
                .help(format!("write `{keyword} NAME: Type = value;`"))
                .emit();
        }

        let ty = ty.unwrap_or_else(|| self.builder.push_type(TypeKind::Error, name_span));
        let init = init.unwrap_or_else(|| {
            let span = self.current_span();
            self.builder.push_expr(ExprKind::Error, span)
        });

        if !self.eat(TokenKind::Semicolon) {
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1002,
                self.current_span(),
                format!("expected `;` after the `{keyword}` declaration, found {found}"),
            )
            .help("end the declaration with `;`")
            .emit();
        }

        ValueDecl {
            name,
            name_span,
            ty,
            init,
        }
    }

    /// Rejects `const mut`, which is not a declaration form.
    pub(crate) fn reject_const_mut(&mut self) {
        if !self.at(TokenKind::Mut) {
            return;
        }
        let token = self.bump();
        self.error(
            codes::E1013,
            token.span,
            "`const` cannot be declared mutable",
        )
        .help("only `static mut` declares writable global storage")
        .emit();
        while self.at(TokenKind::Mut) {
            self.bump();
        }
    }

    /// Rejects a second `mut` on a static declaration.
    pub(crate) fn reject_extra_mut(&mut self) {
        if !self.at(TokenKind::Mut) {
            return;
        }
        let token = self.bump();
        self.error(codes::E1013, token.span, "`static` takes at most one `mut`")
            .help("write `static mut NAME: Type = value;`")
            .emit();
        while self.at(TokenKind::Mut) {
            self.bump();
        }
    }

    /// Reports a block-scope declaration that cannot exist there and skips its
    /// tokens, keeping the enclosing block's closing brace for the block.
    pub(crate) fn recover_local_item(&mut self, start: Span, construct: &str) -> Span {
        self.error(
            codes::E1014,
            start,
            format!("`{construct}` cannot be declared inside a block"),
        )
        .help("move the declaration to file scope")
        .emit();
        self.skip_declaration();
        start.cover(self.previous_span())
    }

    /// Skips a declaration after a diagnostic, including any balanced body.
    pub(crate) fn skip_declaration(&mut self) {
        let mut depth = 0usize;
        while !self.is_eof() {
            match self.current_kind() {
                TokenKind::LBrace => {
                    depth += 1;
                    self.bump();
                }
                TokenKind::RBrace if depth == 0 => break,
                TokenKind::RBrace => {
                    depth -= 1;
                    self.bump();
                    if depth == 0 {
                        break;
                    }
                }
                TokenKind::Semicolon if depth == 0 => {
                    self.bump();
                    break;
                }
                _ => {
                    self.bump();
                }
            }
        }
    }

    /// Parses a function declaration.
    ///
    /// `start` is the first token of the declaration: `const` for a `const
    /// func`, `unsafe` for an `unsafe func`, and either marker for a function
    /// that carries both. The item span covers it so the whole declaration has
    /// one exact source range.
    fn parse_function(&mut self, is_const: bool, is_unsafe: bool, start: Span) -> ItemId {
        let _keyword = self.bump();
        let (name, name_span, generics, params, ret) = self.parse_signature();
        let where_clause = self.parse_where_clause();
        let body = if self.at(TokenKind::LBrace) {
            self.parse_block()
        } else {
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1002,
                self.current_span(),
                format!("expected `{{` to start the function body, found {found}"),
            )
            .help("a function declaration needs a body")
            .emit();
            let span = self.current_span();
            self.builder.push_block(span, Vec::new(), None)
        };

        let span = start.cover(self.previous_span());
        self.builder.push_item(
            ItemKind::Func(FnDecl {
                name,
                name_span,
                is_const,
                is_unsafe,
                generics,
                params,
                ret,
                where_clause,
                body,
            }),
            span,
        )
    }

    /// Parses the name, parameters, and return type shared by every function
    /// declaration, including trait method signatures.
    ///
    /// Parameters stop at the closing parenthesis, the arrow, a `{`, or a `;`,
    /// so a missing `)` in a trait signature does not swallow the semicolon that
    /// ends the declaration.
    fn parse_signature(&mut self) -> (Symbol, Span, GenericParams, Vec<ParamId>, Option<TypeId>) {
        let (name, name_span) = self.expect_ident("a function name");
        let generics = self.parse_generic_params();
        let _ = self.expect(TokenKind::LParen, "`(` after the function name");

        let mut params: Vec<ParamId> = Vec::new();
        while !self.at(TokenKind::RParen)
            && !self.at(TokenKind::Arrow)
            && !self.at(TokenKind::LBrace)
            && !self.at(TokenKind::Semicolon)
            && !self.is_eof()
        {
            let before = self.index;
            if self.at(TokenKind::DotDotDot) {
                self.error(
                    codes::E3088,
                    self.current_span(),
                    "a Seal function cannot declare a variadic parameter",
                )
                .help("`...` belongs to an `extern` declaration; Seal functions take a fixed list")
                .emit();
                while !self.at(TokenKind::RParen)
                    && !self.at(TokenKind::Arrow)
                    && !self.at(TokenKind::LBrace)
                    && !self.at(TokenKind::Semicolon)
                    && !self.is_eof()
                {
                    self.bump();
                }
                break;
            }
            let (param_name, param_name_span) = self.expect_ident("a parameter name");
            let _ = self.expect(TokenKind::Colon, "`:` after the parameter name");
            let ty = self.parse_type();
            let span = param_name_span.cover(self.builder.ast().ty(ty).span);
            params.push(
                self.builder
                    .push_param(param_name, param_name_span, ty, span),
            );

            if self.eat(TokenKind::Comma) {
                continue;
            }
            if self.at(TokenKind::RParen)
                || self.at(TokenKind::Arrow)
                || self.at(TokenKind::LBrace)
                || self.at(TokenKind::Semicolon)
                || self.is_eof()
            {
                break;
            }
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1002,
                self.current_span(),
                format!("expected `,` between parameters, found {found}"),
            )
            .help("separate parameters with `,`")
            .emit();
            if !matches!(self.current_kind(), TokenKind::Ident(_)) {
                break;
            }
            self.ensure_progress(before);
        }

        let _ = self.expect(TokenKind::RParen, "`)` after the parameter list");
        let ret = if self.eat(TokenKind::Arrow) {
            Some(self.parse_type())
        } else {
            None
        };
        (name, name_span, generics, params, ret)
    }

    /// Parses `impl Type { func ... }` and `impl Trait for Type { func ... }`.
    ///
    /// Functions are parsed by the regular function parser and kept in the item
    /// arena; the implementation item references them, keeping the whole tree
    /// reachable without a second declaration grammar.
    fn parse_impl(&mut self) -> ItemId {
        let keyword = self.bump();
        let generics = self.parse_generic_params();
        let (trait_ref, self_ty) = if self.starts_trait_path_before_for() {
            let trait_ref = self.parse_trait_path();
            let _ = self.expect(TokenKind::For, "`for` after the trait name");
            (Some(trait_ref), self.parse_type())
        } else {
            let first = self.parse_type();
            if self.eat(TokenKind::For) {
                (Some(self.trait_path_from_type(first)), self.parse_type())
            } else {
                (None, first)
            }
        };
        let where_clause = self.parse_where_clause();
        let _ = self.expect(TokenKind::LBrace, "`{` after the impl type");

        let mut functions = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.is_eof() {
            let before = self.index;
            if self.at_extern_item() {
                let span = self.current_span();
                self.error(
                    codes::E3092,
                    span,
                    "an `extern` declaration cannot be nested in an impl block",
                )
                .help("declare external items at file scope")
                .emit();
                self.skip_declaration();
            } else if self.at(TokenKind::Func) {
                let start = self.current_span();
                functions.push(self.parse_function(false, false, start));
            } else if self.at(TokenKind::Unsafe) {
                let start = self.current_span();
                let marker = self.bump();
                if self.at(TokenKind::Func) {
                    functions.push(self.parse_function(false, true, start));
                } else {
                    self.error(
                        codes::E3071,
                        marker.span,
                        "`unsafe` applies only to functions",
                    )
                    .help("write `unsafe func name(...) { ... }`")
                    .emit();
                    self.skip_declaration();
                }
            } else {
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E1001,
                    self.current_span(),
                    format!("expected `func` in the impl block, found {found}"),
                )
                .help("an inherent impl only contains function declarations")
                .emit();
                while !self.is_eof() && !self.at(TokenKind::Func) && !self.at(TokenKind::RBrace) {
                    self.bump();
                }
            }
            self.ensure_progress(before);
        }

        let _ = self.expect(TokenKind::RBrace, "`}` to close the impl block");
        let span = keyword.span.cover(self.previous_span());
        self.builder.push_item(
            ItemKind::Impl(ImplDecl {
                generics,
                self_ty,
                trait_ref,
                where_clause,
                functions,
            }),
            span,
        )
    }

    /// Parses `trait Name { func signature; ... }`.
    ///
    /// A trait contains method declarations, each a signature followed by
    /// either `;` or a default body. They are stored as items so the arena owns
    /// them and the reachability invariant covers the whole tree.
    fn parse_trait(&mut self) -> ItemId {
        let keyword = self.bump();
        let (name, name_span) = self.expect_ident("a trait name");
        let generics = self.parse_generic_params();
        let mut supertraits = Vec::new();
        if self.eat(TokenKind::Colon) {
            loop {
                supertraits.push(self.parse_trait_path());
                if !self.eat(TokenKind::Plus) {
                    break;
                }
            }
        }
        let where_clause = self.parse_where_clause();
        let _ = self.expect(TokenKind::LBrace, "`{` after the trait name");

        let mut methods = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.is_eof() {
            let before = self.index;
            if self.at_extern_item() {
                let span = self.current_span();
                self.error(
                    codes::E3092,
                    span,
                    "an `extern` declaration cannot be nested in a trait",
                )
                .help("declare external items at file scope")
                .emit();
                self.skip_declaration();
            } else if self.at(TokenKind::Func) {
                methods.push(self.parse_trait_method(false, self.current_span()));
            } else if self.at(TokenKind::Unsafe) {
                let start = self.current_span();
                let marker = self.bump();
                if self.at(TokenKind::Func) {
                    methods.push(self.parse_trait_method(true, start));
                } else {
                    self.error(
                        codes::E3071,
                        marker.span,
                        "`unsafe` applies only to functions",
                    )
                    .help("write `unsafe func name(...)` in the trait")
                    .emit();
                    self.skip_declaration();
                }
            } else {
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E1001,
                    self.current_span(),
                    format!("expected `func` in the trait, found {found}"),
                )
                .help("a trait only contains method declarations")
                .emit();
                while !self.is_eof()
                    && !self.at(TokenKind::Func)
                    && !self.at(TokenKind::Unsafe)
                    && !self.at(TokenKind::RBrace)
                {
                    // A stray block owns its closing brace. Skipping it as a
                    // unit keeps that brace from closing the trait early and
                    // losing the next method.
                    if self.at(TokenKind::LBrace) {
                        let _ = self.parse_block();
                    } else {
                        self.bump();
                    }
                }
            }
            self.ensure_progress(before);
        }

        let _ = self.expect(TokenKind::RBrace, "`}` to close the trait");
        let span = keyword.span.cover(self.previous_span());
        self.builder.push_item(
            ItemKind::Trait(TraitDecl {
                name,
                name_span,
                generics,
                supertraits,
                where_clause,
                methods,
            }),
            span,
        )
    }

    /// Parses one trait method declaration.
    ///
    /// The declaration is `func name(...) -> Type;` when each implementation
    /// supplies the body, or the same signature followed by a block when the
    /// trait provides a default.
    fn parse_trait_method(&mut self, is_unsafe: bool, start: Span) -> ItemId {
        let _keyword = self.bump();
        let (name, name_span, generics, params, ret) = self.parse_signature();
        let where_clause = self.parse_where_clause();

        let body = if self.eat(TokenKind::Semicolon) {
            None
        } else if self.at(TokenKind::LBrace) {
            Some(self.parse_block())
        } else {
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1002,
                self.current_span(),
                format!(
                    "expected `;` or a default body after the trait method signature, found {found}"
                ),
            )
            .help("end a signature-only declaration with `;`, or write the default body")
            .emit();
            None
        };

        let span = start.cover(self.previous_span());
        self.builder.push_item(
            ItemKind::TraitMethod(TraitMethodDecl {
                name,
                name_span,
                is_unsafe,
                generics,
                params,
                ret,
                where_clause,
                body,
            }),
            span,
        )
    }

    fn parse_struct(&mut self) -> ItemId {
        let keyword = self.bump();
        let (name, name_span) = self.expect_ident("a struct name");
        let generics = self.parse_generic_params();
        let where_clause = self.parse_where_clause();
        self.parse_struct_body(keyword.span, name, name_span, generics, where_clause, false)
    }

    fn parse_enum(&mut self) -> ItemId {
        let keyword = self.bump();
        let (name, name_span) = self.expect_ident("an enum name");
        let generics = self.parse_generic_params();
        let where_clause = self.parse_where_clause();
        let _ = self.expect(TokenKind::LBrace, "`{` after the enum name");

        let mut variants = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.is_eof() {
            let before = self.index;
            variants.push(self.parse_enum_variant());

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
                format!("expected `,` between variants, found {found}"),
            )
            .help("separate variants with `,`")
            .emit();
            if !matches!(self.current_kind(), TokenKind::Ident(_)) {
                break;
            }
            self.ensure_progress(before);
        }

        let _ = self.expect(TokenKind::RBrace, "`}` to close the enum");
        let span = keyword.span.cover(self.previous_span());
        self.builder.push_item(
            ItemKind::Enum(EnumDecl {
                name,
                name_span,
                generics,
                where_clause,
                variants,
            }),
            span,
        )
    }

    /// Parses one variant: a name, optionally followed by positional or named
    /// fields.
    fn parse_enum_variant(&mut self) -> VariantDeclId {
        let (name, name_span) = self.expect_ident("a variant name");
        let fields = if self.at(TokenKind::LParen) {
            let tuple_fields = self.parse_tuple_variant_fields();
            if self.at(TokenKind::LBrace) {
                self.error(
                    codes::E1010,
                    self.current_span(),
                    "a variant cannot have both positional and named fields",
                )
                .help("choose `Variant(Type, ...)` or `Variant { name: Type, ... }`")
                .emit();
                self.skip_variant_braces();
            }
            if tuple_fields.is_empty() {
                VariantFieldList::Unit
            } else {
                VariantFieldList::Tuple(tuple_fields)
            }
        } else if self.at(TokenKind::LBrace) {
            VariantFieldList::Named(self.parse_named_variant_fields())
        } else {
            VariantFieldList::Unit
        };

        let span = name_span.cover(self.previous_span());
        self.builder.push_variant(name, name_span, fields, span)
    }

    /// Parses a positional field list, starting at `(`.
    ///
    /// A positional field has only a type. A written name is diagnosed and then
    /// skipped, so the rest of the list still parses into a usable declaration.
    fn parse_tuple_variant_fields(&mut self) -> Vec<TupleFieldDeclId> {
        self.bump();
        let mut fields = Vec::new();
        while !self.at(TokenKind::RParen) && !self.is_eof() {
            let before = self.index;
            if matches!(self.current_kind(), TokenKind::Ident(_))
                && self.peek_kind() == Some(TokenKind::Colon)
            {
                let field_name = self.bump();
                self.bump();
                self.error(
                    codes::E1010,
                    field_name.span.cover(self.previous_span()),
                    "a positional field has no name",
                )
                .help("write only the field type, such as `Some(i32)`")
                .emit();
            }
            let ty = self.parse_type();
            let span = self.builder.ast().ty(ty).span;
            fields.push(self.builder.push_tuple_field(ty, span));

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
                format!("expected `,` or `)` in the positional field list, found {found}"),
            )
            .help("separate field types with `,`")
            .emit();
            if !is_type_start(self.current_kind()) {
                break;
            }
            self.ensure_progress(before);
        }
        let _ = self.expect(TokenKind::RParen, "`)` to close the positional fields");
        if fields.is_empty() {
            self.error(
                codes::E1010,
                self.previous_span(),
                "an empty positional field list is written as a unit variant",
            )
            .help("remove the parentheses, as in `Variant`")
            .emit();
        }
        fields
    }

    /// Parses a named field list, starting at `{`.
    fn parse_named_variant_fields(&mut self) -> Vec<FieldDeclId> {
        let _ = self.expect(TokenKind::LBrace, "`{` to start the variant fields");
        let mut fields = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.is_eof() {
            let before = self.index;
            let (field_name, field_name_span) = self.expect_ident("a field name");
            let _ = self.expect(TokenKind::Colon, "`:` after the field name");
            let ty = self.parse_type();
            let field_span = field_name_span.cover(self.builder.ast().ty(ty).span);
            fields.push(
                self.builder
                    .push_field(field_name, field_name_span, ty, field_span),
            );
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
                format!("expected `,` between fields, found {found}"),
            )
            .help("separate fields with `,`")
            .emit();
            if !matches!(self.current_kind(), TokenKind::Ident(_)) {
                break;
            }
            self.ensure_progress(before);
        }
        let _ = self.expect(TokenKind::RBrace, "`}` to close the variant");
        fields
    }

    /// Skips a discarded braced field list after a mixed-form diagnostic.
    ///
    /// The nodes cannot be attached to the variant because the positional form
    /// already owns its fields, and leaving them in the AST arena would violate
    /// the reachability invariant. Balanced skipping keeps recovery local.
    fn skip_variant_braces(&mut self) {
        if !self.eat(TokenKind::LBrace) {
            return;
        }
        let mut depth = 1usize;
        while !self.is_eof() && depth > 0 {
            match self.current_kind() {
                TokenKind::LBrace => {
                    depth += 1;
                    self.bump();
                }
                TokenKind::RBrace => {
                    depth -= 1;
                    self.bump();
                }
                _ => {
                    self.bump();
                }
            }
        }
        if depth > 0 {
            self.error(
                codes::E1002,
                self.current_span(),
                "expected `}` to close the variant fields",
            )
            .help("add the missing `}`")
            .emit();
        }
    }

    /// Returns the current token; at or past the end, a synthetic `Eof` tied to
    /// the previous token's position.
    pub(crate) fn current(&self) -> Token {
        self.pending
            .or_else(|| self.tokens.get(self.index).copied())
            .unwrap_or(Token {
                kind: TokenKind::Eof,
                span: self.previous,
                preceded_by_newline: false,
            })
    }

    #[inline]
    pub(crate) fn current_kind(&self) -> TokenKind {
        self.current().kind
    }

    #[inline]
    pub(crate) fn current_span(&self) -> Span {
        self.current().span
    }

    /// Returns the kind of the token after the current one, if any.
    #[inline]
    pub(crate) fn peek_kind(&self) -> Option<TokenKind> {
        let offset = if self.pending.is_some() { 0 } else { 1 };
        self.tokens.get(self.index + offset).map(|token| token.kind)
    }

    #[inline]
    pub(crate) fn previous_span(&self) -> Span {
        self.previous
    }

    /// Returns `true` at end of file, including a malformed stream without `Eof`.
    #[inline]
    pub(crate) fn is_eof(&self) -> bool {
        matches!(self.current_kind(), TokenKind::Eof)
    }

    /// Returns `true` if the current token has the given data-less kind.
    #[inline]
    pub(crate) fn at(&self, kind: TokenKind) -> bool {
        self.current_kind() == kind
    }

    /// Returns `true` when the current token is the identifier `name`.
    ///
    /// Contextual keywords are spellings, not token kinds, so a parser position
    /// asks for the spelling and leaves the name usable everywhere else.
    pub(crate) fn at_name(&self, name: &str) -> bool {
        match self.current_kind() {
            TokenKind::Ident(symbol) => self.interner.resolve(symbol) == name,
            _ => false,
        }
    }

    /// Consumes and returns the current token. `Eof` is never consumed.
    pub(crate) fn bump(&mut self) -> Token {
        if let Some(token) = self.pending.take() {
            self.previous = token.span;
            return token;
        }
        let token = self.current();
        if !matches!(token.kind, TokenKind::Eof) && self.index < self.tokens.len() {
            self.index += 1;
        }
        self.previous = token.span;
        token
    }

    /// Consumes the current token if it has `kind`.
    pub(crate) fn eat(&mut self, kind: TokenKind) -> bool {
        if self.at(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    /// Requires `kind`, reporting E1002 if it is missing. Does not consume a
    /// mismatched token, so the caller's recovery can decide what to do.
    pub(crate) fn expect(&mut self, kind: TokenKind, expected: &str) -> bool {
        if self.eat(kind) {
            true
        } else {
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1002,
                self.current_span(),
                format!("expected {expected}, found {found}"),
            )
            .help(format!("expected {expected} here"))
            .emit();
            false
        }
    }

    /// Requires an identifier and returns its symbol and span.
    pub(crate) fn expect_ident(&mut self, context: &str) -> (Symbol, Span) {
        let token = self.current();
        match token.kind {
            TokenKind::Ident(symbol) => {
                self.bump();
                (symbol, token.span)
            }
            _ => {
                let found = self.describe(token.kind);
                let help = match token_name(token.kind) {
                    Some(keyword) => {
                        format!("`{keyword}` is a reserved keyword; rename the identifier")
                    }
                    None => "identifiers start with a letter or `_`".to_owned(),
                };
                self.error(
                    codes::E1005,
                    token.span,
                    format!("expected {context}, found {found}"),
                )
                .help(help)
                .emit();
                (Symbol::EMPTY, token.span)
            }
        }
    }

    /// Builds a diagnostic in this file's position space.
    pub(crate) fn error(
        &self,
        code: ErrorCode,
        span: Span,
        message: impl Into<String>,
    ) -> ErrorBuilder<'_> {
        self.dcx
            .error(message)
            .code(code.diag_code())
            .primary(span, "")
    }

    /// Reports a bounded-recursion failure.
    pub(crate) fn report_depth(&self, span: Span) {
        self.error(
            codes::E1008,
            span,
            "expression or type is nested too deeply",
        )
        .help("simplify it or split it into several `let` bindings")
        .emit();
    }

    /// Builds the error expression used when the depth limit is hit, consuming
    /// one token so the caller always makes progress.
    pub(crate) fn depth_error_expr(&mut self, span: Span) -> ExprId {
        self.report_depth(span);
        if !self.is_eof() {
            self.bump();
        }
        let span = span.cover(self.previous_span());
        self.builder.push_expr(seal_ast::ExprKind::Error, span)
    }

    /// Builds the error type used when the depth limit is hit.
    pub(crate) fn depth_error_type(&mut self, span: Span) -> TypeId {
        self.report_depth(span);
        self.builder.push_type(seal_ast::TypeKind::Error, span)
    }

    /// Parses a fixed array length, written as an integer literal whose type
    /// is `usize` (either unsuffixed or suffixed `usize`).
    ///
    /// Array lengths are type-level constants; Seal does not have `const`
    /// items yet, so the grammar accepts a literal instead of an expression.
    pub(crate) fn parse_array_length(&mut self, context: &str) -> (u64, Span) {
        let token = self.current();
        let TokenKind::Int { value, suffix, .. } = token.kind else {
            let found = self.describe(token.kind);
            self.error(
                codes::E1009,
                token.span,
                format!("expected an array length, found {found}"),
            )
            .help(format!(
                "write the {context} as a non-negative integer literal"
            ))
            .emit();
            return (0, token.span);
        };
        self.bump();

        if let Some(suffix) = suffix
            && suffix.as_str() != "usize"
        {
            self.error(
                codes::E1009,
                token.span,
                format!("an array length must be `usize`, not `{}`", suffix.as_str()),
            )
            .help("remove the suffix, or write `usize`")
            .emit();
        }

        if value > u128::from(u32::MAX) {
            self.error(codes::E1009, token.span, "array length is too large")
                .help(format!(
                    "the maximum supported array length is {} elements",
                    u32::MAX
                ))
                .emit();
            return (0, token.span);
        }

        (value as u64, token.span)
    }

    /// Advances one token if the caller consumed nothing, preventing hangs.
    pub(crate) fn ensure_progress(&mut self, before: usize) {
        if self.index == before && !self.is_eof() {
            self.bump();
        }
    }

    /// Returns a human-readable description of a token kind for diagnostics.
    pub(crate) fn describe(&self, kind: TokenKind) -> String {
        match kind {
            TokenKind::Ident(symbol) => format!("`{}`", self.interner.resolve(symbol)),
            TokenKind::Int { .. } => "a number".to_owned(),
            TokenKind::Float { .. } => "a floating-point number".to_owned(),
            TokenKind::Str(_) => "a string literal".to_owned(),
            TokenKind::CStr(_) => "a C string literal".to_owned(),
            TokenKind::Char(_) => "a character literal".to_owned(),
            TokenKind::Eof => "end of file".to_owned(),
            TokenKind::Error => "a malformed token".to_owned(),
            other => match token_name(other) {
                Some(name) => format!("`{name}`"),
                None => "a token".to_owned(),
            },
        }
    }
}

fn token_name(kind: TokenKind) -> Option<&'static str> {
    Some(match kind {
        TokenKind::Func => "func",
        TokenKind::Struct => "struct",
        TokenKind::Impl => "impl",
        TokenKind::Trait => "trait",
        TokenKind::Dyn => "dyn",
        TokenKind::For => "for",
        TokenKind::In => "in",
        TokenKind::If => "if",
        TokenKind::Else => "else",
        TokenKind::Let => "let",
        TokenKind::Mut => "mut",
        TokenKind::While => "while",
        TokenKind::Match => "match",
        TokenKind::Break => "break",
        TokenKind::Continue => "continue",
        TokenKind::Return => "return",
        TokenKind::True => "true",
        TokenKind::False => "false",
        TokenKind::Where => "where",
        TokenKind::Const => "const",
        TokenKind::Static => "static",
        TokenKind::Unsafe => "unsafe",
        TokenKind::LParen => "(",
        TokenKind::RParen => ")",
        TokenKind::LBrace => "{",
        TokenKind::RBrace => "}",
        TokenKind::LBracket => "[",
        TokenKind::RBracket => "]",
        TokenKind::Comma => ",",
        TokenKind::Semicolon => ";",
        TokenKind::Colon => ":",
        TokenKind::ColonColon => "::",
        TokenKind::Dot => ".",
        TokenKind::DotDotDot => "...",
        TokenKind::Arrow => "->",
        TokenKind::FatArrow => "=>",
        TokenKind::Plus => "+",
        TokenKind::Minus => "-",
        TokenKind::Star => "*",
        TokenKind::Slash => "/",
        TokenKind::Percent => "%",
        TokenKind::PlusPlus => "++",
        TokenKind::MinusMinus => "--",
        TokenKind::Eq => "=",
        TokenKind::EqEq => "==",
        TokenKind::NotEq => "!=",
        TokenKind::Lt => "<",
        TokenKind::Gt => ">",
        TokenKind::LtEq => "<=",
        TokenKind::GtEq => ">=",
        TokenKind::AndAnd => "&&",
        TokenKind::OrOr => "||",
        TokenKind::Bang => "!",
        TokenKind::Amp => "&",
        TokenKind::Pipe => "|",
        TokenKind::Caret => "^",
        TokenKind::Tilde => "~",
        TokenKind::Shl => "<<",
        TokenKind::Shr => ">>",
        TokenKind::PlusEq => "+=",
        TokenKind::MinusEq => "-=",
        TokenKind::StarEq => "*=",
        TokenKind::SlashEq => "/=",
        TokenKind::PercentEq => "%=",
        TokenKind::AmpEq => "&=",
        TokenKind::PipeEq => "|=",
        TokenKind::CaretEq => "^=",
        TokenKind::ShlEq => "<<=",
        TokenKind::ShrEq => ">>=",
        _ => return None,
    })
}

/// Returns `true` when a token can start a type.
fn is_type_start(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Star | TokenKind::LBracket | TokenKind::Ident(_)
    )
}

/// Returns `true` when a token cannot start an expression and parsing should
/// stop without consuming it.
pub(crate) fn is_expression_stop(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Eof
            | TokenKind::LBrace
            | TokenKind::RBrace
            | TokenKind::RParen
            | TokenKind::RBracket
            | TokenKind::Semicolon
            | TokenKind::Comma
            | TokenKind::Colon
            | TokenKind::Arrow
            | TokenKind::FatArrow
    )
}

/// Returns `true` when a token can start an expression, used by recovery.
pub(crate) fn can_start_expr(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Ident(_)
            | TokenKind::Int { .. }
            | TokenKind::Float { .. }
            | TokenKind::Str(_)
            | TokenKind::CStr(_)
            | TokenKind::Char(_)
            | TokenKind::True
            | TokenKind::False
            | TokenKind::Minus
            | TokenKind::Bang
            | TokenKind::Tilde
            | TokenKind::Amp
            | TokenKind::Star
            | TokenKind::PlusPlus
            | TokenKind::MinusMinus
            | TokenKind::LParen
            | TokenKind::LBracket
            | TokenKind::If
            | TokenKind::Match
            | TokenKind::Unsafe
            | TokenKind::Error
    )
}
