//! Parser state, token access, diagnostics, and top-level items.

use seal_ast::{
    Ast, AstBuilder, EnumDecl, ExprId, FieldDeclId, FnDecl, GenericParams, ImplDecl, ItemId,
    ItemKind, ParamId, StructDecl, TraitDecl, TraitMethodDecl, TupleFieldDeclId, TypeId,
    VariantDeclId, VariantFieldList,
};
use seal_diagnostics::{DiagCtxt, ErrorBuilder};
use seal_error_codes as codes;
use seal_error_codes::ErrorCode;
use seal_lexer::{Lexed, Token, TokenKind};
use seal_span::Span;
use seal_symbol::{Interner, Symbol};

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
                let item = self.parse_function();
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
            } else {
                let start = self.current_span();
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E1001,
                    start,
                    format!("expected an item, found {found}"),
                )
                .help("top-level items start with `func`, `struct`, `enum`, `impl`, or `trait`")
                .emit();
                while !self.is_eof()
                    && !self.at(TokenKind::Func)
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

    fn parse_function(&mut self) -> ItemId {
        let keyword = self.bump();
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

        let span = keyword.span.cover(self.previous_span());
        self.builder.push_item(
            ItemKind::Func(FnDecl {
                name,
                name_span,
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
            if self.at(TokenKind::Func) {
                functions.push(self.parse_function());
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
    /// A trait contains only signatures. They are stored as items so the arena
    /// owns them and the reachability invariant covers the whole tree.
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
            if self.at(TokenKind::Func) {
                methods.push(self.parse_trait_method());
            } else {
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E1001,
                    self.current_span(),
                    format!("expected `func` in the trait, found {found}"),
                )
                .help("a trait only contains method signatures")
                .emit();
                while !self.is_eof() && !self.at(TokenKind::Func) && !self.at(TokenKind::RBrace) {
                    self.bump();
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

    /// Parses one `func name(...) -> Type;` trait method signature.
    fn parse_trait_method(&mut self) -> ItemId {
        let keyword = self.bump();
        let (name, name_span, generics, params, ret) = self.parse_signature();
        let where_clause = self.parse_where_clause();

        if !self.eat(TokenKind::Semicolon) {
            let found = self.describe(self.current_kind());
            if self.at(TokenKind::LBrace) {
                self.error(
                    codes::E1002,
                    self.current_span(),
                    "a trait method declares only its signature, not a body",
                )
                .help("end the declaration with `;`; the implementation supplies the body")
                .emit();
                // Consume the body so its braces cannot open a second item and
                // so recovery resumes after the method.
                let _ = self.parse_block();
            } else {
                self.error(
                    codes::E1002,
                    self.current_span(),
                    format!("expected `;` after the trait method signature, found {found}"),
                )
                .help("a trait method declaration ends with `;`")
                .emit();
            }
        }

        let span = keyword.span.cover(self.previous_span());
        self.builder.push_item(
            ItemKind::TraitMethod(TraitMethodDecl {
                name,
                name_span,
                generics,
                params,
                ret,
                where_clause,
            }),
            span,
        )
    }

    fn parse_struct(&mut self) -> ItemId {
        let keyword = self.bump();
        let (name, name_span) = self.expect_ident("a struct name");
        let generics = self.parse_generic_params();
        let where_clause = self.parse_where_clause();
        let _ = self.expect(TokenKind::LBrace, "`{` after the struct name");

        let mut fields = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.is_eof() {
            let before = self.index;
            let (field_name, field_name_span) = self.expect_ident("a field name");
            let _ = self.expect(TokenKind::Colon, "`:` after the field name");
            let ty = self.parse_type();
            let span = field_name_span.cover(self.builder.ast().ty(ty).span);
            fields.push(
                self.builder
                    .push_field(field_name, field_name_span, ty, span),
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

        let _ = self.expect(TokenKind::RBrace, "`}` to close the struct");
        let span = keyword.span.cover(self.previous_span());
        self.builder.push_item(
            ItemKind::Struct(StructDecl {
                name,
                name_span,
                generics,
                where_clause,
                fields,
            }),
            span,
        )
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
                self.error(
                    codes::E1005,
                    token.span,
                    format!("expected {context}, found {found}"),
                )
                .help("identifiers start with a letter or `_`")
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
            | TokenKind::Error
    )
}
