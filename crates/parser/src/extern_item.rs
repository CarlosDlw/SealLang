//! `extern` declarations.
//!
//! The parser accepts the ABI spelling as written; lowering decides whether the
//! compiler supports it. Members reuse the ordinary parameter and type grammar,
//! so an external signature is never a second kind of type syntax.

use seal_ast::{
    ExternBlockDecl, ExternFnDecl, ExternStaticDecl, ItemId, ItemKind, LinkDirective, LinkName,
    ParamId, TypeId,
};
use seal_error_codes as codes;
use seal_lexer::TokenKind;
use seal_span::Span;
use seal_symbol::Symbol;

use crate::parser::Parser;

impl Parser<'_> {
    /// Returns `true` at the start of an `extern "..."` item.
    ///
    /// `extern` is contextual: it is a name everywhere else, so only the pair
    /// `extern` followed by a string starts a declaration.
    pub(crate) fn at_extern_item(&self) -> bool {
        self.at_name("extern") && matches!(self.peek_kind(), Some(TokenKind::Str(_)))
    }

    /// Parses one `extern` item, either a block or a single declaration.
    pub(crate) fn parse_extern_item(&mut self) -> ItemId {
        let start = self.current_span();
        self.bump();
        let (abi, abi_span) = self.parse_abi();

        if self.at(TokenKind::LBrace) {
            let (items, links) = self.parse_extern_block_members(abi, abi_span);
            let span = start.cover(self.previous_span());
            return self.builder.push_item(
                ItemKind::ExternBlock(ExternBlockDecl {
                    abi,
                    abi_span,
                    links,
                    items,
                }),
                span,
            );
        }
        if self.at(TokenKind::Func) {
            return self.parse_extern_function(abi, abi_span, start);
        }
        if self.at(TokenKind::Static) {
            return self.parse_extern_static(abi, abi_span, start);
        }
        if self.at(TokenKind::Struct) || self.at_name("type") {
            self.error(
                codes::E3081,
                self.current_span(),
                "an external aggregate is not supported yet",
            )
            .help("declare only `func` and `static` items in an `extern` block")
            .emit();
            self.skip_declaration();
            let span = start.cover(self.previous_span());
            return self.builder.push_item(ItemKind::Error, span);
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
        self.builder.push_item(ItemKind::Error, span)
    }

    /// Parses the ABI string after `extern`.
    fn parse_abi(&mut self) -> (Symbol, Span) {
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
        while !self.at(TokenKind::RParen) && !self.is_eof() {
            let before = self.index;
            if self.at(TokenKind::DotDot) && self.peek_kind() == Some(TokenKind::Dot) {
                let span = self.current_span();
                self.bump();
                self.bump();
                self.error(
                    codes::E3088,
                    span,
                    "variadic declarations are not supported yet",
                )
                .help("declare a fixed signature; variadic calls arrive in a later release")
                .emit();
                variadic = true;
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
        self.builder.push_item(
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
        self.builder.push_item(
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
        )
    }
}
