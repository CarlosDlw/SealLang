//! `extern` declarations.
//!
//! The parser accepts the ABI spelling as written; lowering decides whether the
//! compiler supports it. Members reuse the ordinary parameter and type grammar,
//! so an external signature is never a second kind of type syntax.

use seal_ast::{
    ExternBlockDecl, ExternFnDecl, ExternStaticDecl, ItemId, ItemKind, LinkDirective, LinkName,
    Modifiers, OpaqueTypeDecl, ParamId, StructDecl, TypeId,
};
use seal_error_codes as codes;
use seal_lexer::TokenKind;
use seal_span::Span;
use seal_symbol::Symbol;

use crate::parser::Parser;

impl Parser<'_> {
    /// Returns `true` at the start of an external item.
    ///
    /// `extern` is contextual: it is a name everywhere else, so only a string,
    /// `struct`, or `type` after it starts a declaration.
    pub(crate) fn at_extern_item(&self) -> bool {
        self.at_name("extern")
            && match self.peek_kind() {
                Some(TokenKind::Str(_)) | Some(TokenKind::Struct) => true,
                Some(TokenKind::Ident(name)) => self.interner.resolve(name) == "type",
                _ => false,
            }
    }

    /// Parses one `extern` item, either a block or a single declaration.
    pub(crate) fn parse_extern_item(&mut self) -> ItemId {
        let modifiers = std::mem::take(&mut self.item_modifiers);
        let start = self.current_span();
        self.bump();
        if self.at(TokenKind::Struct) {
            self.item_modifiers = modifiers;
            return self.parse_extern_struct(start);
        }
        if self.at_name("type") {
            self.item_modifiers = modifiers;
            return self.parse_extern_opaque(start);
        }
        let (abi, abi_span) = self.parse_abi();

        if self.at(TokenKind::LBrace) {
            self.item_modifiers = Modifiers::none();
            let (items, links) = self.parse_extern_block_members(abi, abi_span);
            let span = start.cover(self.previous_span());
            return self.builder.push_item_with_modifiers(
                ItemKind::ExternBlock(ExternBlockDecl {
                    abi,
                    abi_span,
                    links,
                    items,
                }),
                span,
                modifiers,
            );
        }
        if self.at(TokenKind::Func) {
            self.item_modifiers = modifiers;
            return self.parse_extern_function(abi, abi_span, start);
        }
        if self.at(TokenKind::Static) {
            self.item_modifiers = modifiers;
            return self.parse_extern_static(abi, abi_span, start);
        }
        let found = self.describe(self.current_kind());
        self.error(
            codes::E3081,
            self.current_span(),
            format!("expected a `func` or `static` declaration, found {found}"),
        )
        .help("write `extern \"c\" func name(...) -> Type;` or `extern \"c\" static name: Type;`")
        .emit();
        self.skip_declaration();
        let span = start.cover(self.previous_span());
        self.builder
            .push_item_with_modifiers(ItemKind::Error, span, Modifiers::none())
    }

    /// Parses `extern struct Name { fields }`.
    fn parse_extern_struct(&mut self, start: Span) -> ItemId {
        let _keyword = self.bump();
        let (name, name_span) = self.expect_ident("a struct name");
        let generics = self.parse_generic_params();
        let where_clause = self.parse_where_clause();
        if !generics.is_empty() || !where_clause.predicates.is_empty() {
            let span = if !generics.is_empty() {
                generics.span
            } else {
                where_clause.span
            };
            self.error(
                codes::E3093,
                span,
                "an external struct cannot declare generic parameters",
            )
            .help("remove the parameters and bounds; a C struct has one concrete layout")
            .emit();
        }
        self.parse_struct_body(start, name, name_span, generics, where_clause, true)
    }

    /// Parses `extern type Name;`.
    fn parse_extern_opaque(&mut self, start: Span) -> ItemId {
        let _keyword = self.bump();
        let (name, name_span) = self.expect_ident("an opaque type name");
        if self.at(TokenKind::Lt) {
            let generics = self.parse_generic_params();
            self.error(
                codes::E3093,
                generics.span,
                "an opaque type cannot declare generic parameters",
            )
            .help("remove the parameters; an external type names one C type")
            .emit();
        }
        if !self.eat(TokenKind::Semicolon) {
            let found = self.describe(self.current_kind());
            self.error(
                codes::E3081,
                self.current_span(),
                format!("an opaque type declaration ends with `;`, found {found}"),
            )
            .help("write `extern type Name;`")
            .emit();
            self.skip_declaration();
        }
        let span = start.cover(self.previous_span());
        self.builder.push_item_with_modifiers(
            ItemKind::OpaqueType(OpaqueTypeDecl { name, name_span }),
            span,
            self.item_modifiers,
        )
    }

    /// Parses the braced field list shared by `struct` and `extern struct`.
    pub(crate) fn parse_struct_body(
        &mut self,
        start: Span,
        name: Symbol,
        name_span: Span,
        generics: seal_ast::GenericParams,
        where_clause: seal_ast::WhereClause,
        is_extern: bool,
    ) -> ItemId {
        let _ = self.expect(TokenKind::LBrace, "`{` after the struct name");
        let mut fields = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.is_eof() {
            let before = self.index;
            let field_modifiers = self.parse_modifiers();
            if field_modifiers.cfg.is_some() {
                self.error(
                    codes::E2085,
                    self.current_span(),
                    "a struct field cannot have a `cfg` condition",
                )
                .help("place `cfg` on the struct or on the code that uses the field")
                .emit();
            }
            if field_modifiers.is_test {
                self.error(
                    codes::E1001,
                    self.current_span(),
                    "a field cannot be a test",
                )
                .help("remove `test`; it applies to functions")
                .emit();
            }
            let (field_name, field_name_span) = self.expect_ident("a field name");
            let _ = self.expect(TokenKind::Colon, "`:` after the field name");
            let ty = self.parse_type();
            let span = field_name_span.cover(self.builder.ast().ty(ty).span);
            fields.push(self.builder.push_field_with_modifiers(
                field_name,
                field_name_span,
                ty,
                span,
                field_modifiers.visibility,
                field_modifiers.cfg,
            ));

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
        let span = start.cover(self.previous_span());
        self.builder.push_item_with_modifiers(
            ItemKind::Struct(StructDecl {
                name,
                name_span,
                is_extern,
                generics,
                where_clause,
                fields,
            }),
            span,
            self.item_modifiers,
        )
    }

    /// Parses the ABI string after `extern`.
    pub(crate) fn parse_abi(&mut self) -> (Symbol, Span) {
        let token = self.current();
        if let TokenKind::Str(symbol) = token.kind {
            self.bump();
            return (symbol, token.span);
        }
        let found = self.describe(token.kind);
        self.error(
            codes::E3081,
            token.span,
            format!("expected an ABI string after `extern`, found {found}"),
        )
        .help("write the ABI as a string, such as `extern \"c\"`")
        .emit();
        (Symbol::EMPTY, token.span)
    }

    /// Parses the braced member list of an external block.
    fn parse_extern_block_members(
        &mut self,
        abi: Symbol,
        abi_span: Span,
    ) -> (Vec<ItemId>, Vec<LinkDirective>) {
        self.bump();
        let mut items = Vec::new();
        let mut links = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.is_eof() {
            let before = self.index;
            if self.at(TokenKind::Func) {
                let start = self.current_span();
                items.push(self.parse_extern_function(abi, abi_span, start));
            } else if self.at(TokenKind::Static) {
                let start = self.current_span();
                items.push(self.parse_extern_static(abi, abi_span, start));
            } else if self.at_extern_item() {
                let span = self.current_span();
                self.error(
                    codes::E3092,
                    span,
                    "an `extern` declaration cannot be nested in another `extern` block",
                )
                .help("declare the ABI directly on the member, or close the block first")
                .emit();
                self.skip_declaration();
            } else if self.at_name("link") {
                if let Some(directive) = self.parse_link_directive() {
                    links.push(directive);
                }
            } else {
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E3081,
                    self.current_span(),
                    format!("an `extern` block only contains declarations, found {found}"),
                )
                .help("declare a `func` or a `static`")
                .emit();
                self.skip_declaration();
            }
            self.ensure_progress(before);
        }
        let _ = self.expect(TokenKind::RBrace, "`}` to close the `extern` block");
        (items, links)
    }

    /// Parses `link "name";` inside an external block.
    fn parse_link_directive(&mut self) -> Option<LinkDirective> {
        let start = self.current_span();
        self.bump();
        let token = self.current();
        let TokenKind::Str(name) = token.kind else {
            let found = self.describe(token.kind);
            self.error(
                codes::E3091,
                token.span,
                format!("expected a library name string after `link`, found {found}"),
            )
            .help("write the library name as a string, such as `link \"m\"`")
            .emit();
            self.skip_declaration();
            return None;
        };
        self.bump();
        if !self.eat(TokenKind::Semicolon) {
            let found = self.describe(self.current_kind());
            self.error(
                codes::E3091,
                self.current_span(),
                format!("a `link` directive ends with `;`, found {found}"),
            )
            .emit();
            self.skip_declaration();
            return None;
        }
        Some(LinkDirective {
            name,
            span: start.cover(self.previous_span()),
        })
    }

    /// Parses one external function declaration.
    fn parse_extern_function(&mut self, abi: Symbol, abi_span: Span, start: Span) -> ItemId {
        let _keyword = self.bump();
        let (name, name_span) = self.expect_ident("a function name");

        if self.at(TokenKind::Lt) {
            let generics = self.parse_generic_params();
            self.error(
                codes::E3093,
                generics.span,
                "an external function cannot declare generic parameters",
            )
            .help("remove the type parameters; C functions have one concrete signature")
            .emit();
        }

        let _ = self.expect(TokenKind::LParen, "`(` after the function name");
        let mut params: Vec<ParamId> = Vec::new();
        let mut variadic = false;
        let mut variadic_span = None;
        while !self.at(TokenKind::RParen) && !self.is_eof() {
            let before = self.index;
            if self.at(TokenKind::DotDotDot) {
                let span = self.current_span();
                self.bump();
                variadic = true;
                variadic_span = Some(span);
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
        if variadic && !self.at(TokenKind::RParen) {
            self.error(
                codes::E3088,
                self.current_span(),
                "`...` must be the last element of the parameter list",
            )
            .help("move `...` to the end, after every named parameter")
            .emit();
            while !self.at(TokenKind::RParen) && !self.at(TokenKind::Semicolon) && !self.is_eof() {
                self.bump();
            }
        }
        if let Some(span) = variadic_span
            && params.is_empty()
        {
            self.error(
                codes::E3088,
                span,
                "a variadic function needs at least one named parameter before `...`",
            )
            .help("declare the fixed parameters C always receives, then `...`")
            .emit();
        }
        let _ = self.expect(TokenKind::RParen, "`)` after the parameter list");

        let ret = if self.eat(TokenKind::Arrow) {
            Some(self.parse_type())
        } else {
            None
        };
        let link_name = self.parse_link_name();

        if !self.eat(TokenKind::Semicolon) {
            let found = self.describe(self.current_kind());
            self.error(
                codes::E3081,
                self.current_span(),
                format!("an external declaration ends with `;`, found {found}"),
            )
            .help("remove the body; an external function is declared, not defined")
            .emit();
            self.skip_declaration();
        }

        let span = start.cover(self.previous_span());
        self.builder.push_item_with_modifiers(
            ItemKind::ExternFunc(ExternFnDecl {
                abi,
                abi_span,
                name,
                name_span,
                params,
                ret,
                link_name,
                variadic,
                span,
            }),
            span,
            self.item_modifiers,
        )
    }

    /// Parses an optional `link_name "..."` clause.
    fn parse_link_name(&mut self) -> Option<LinkName> {
        if !self.at_name("link_name") {
            return None;
        }
        self.bump();
        let token = self.current();
        if let TokenKind::Str(symbol) = token.kind {
            self.bump();
            return Some(LinkName {
                symbol,
                span: token.span,
            });
        }
        let found = self.describe(token.kind);
        self.error(
            codes::E3081,
            token.span,
            format!("expected a symbol string after `link_name`, found {found}"),
        )
        .help("write the linker symbol as a string, such as `link_name \"abs\"`")
        .emit();
        None
    }

    /// Parses one external static declaration.
    fn parse_extern_static(&mut self, abi: Symbol, abi_span: Span, start: Span) -> ItemId {
        let _keyword = self.bump();
        let mutable = self.eat(TokenKind::Mut);
        let (name, name_span) = self.expect_ident("a static name");
        let _ = self.expect(TokenKind::Colon, "`:` after the static name");
        let ty: TypeId = self.parse_type();
        if !self.eat(TokenKind::Semicolon) {
            let found = self.describe(self.current_kind());
            self.error(
                codes::E3081,
                self.current_span(),
                format!("an external static ends with `;`, found {found}"),
            )
            .help("remove the initializer; an external static has storage defined by C")
            .emit();
            self.skip_declaration();
        }
        let span = start.cover(self.previous_span());
        self.builder.push_item_with_modifiers(
            ItemKind::ExternStatic(ExternStaticDecl {
                abi,
                abi_span,
                name,
                name_span,
                mutable,
                ty,
                span,
            }),
            span,
            self.item_modifiers,
        )
    }
}
