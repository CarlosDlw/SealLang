//! Parsing for `module` declarations and `use` imports.
//!
//! Both constructs are file-scoped syntax. A module declaration names the
//! namespace this file contributes to; imports are absolute and are collected
//! before any name resolution so dependency cycles between modules stay legal.

use seal_ast::{
    ItemId, Modifiers, ModuleDecl, UseDecl, UseItem, UsePath, UseRoot, UseSegment, Visibility,
};
use seal_error_codes as codes;
use seal_lexer::TokenKind;
use seal_span::Span;
use seal_symbol::Symbol;

use crate::parser::Parser;

impl Parser<'_> {
    /// Parses `module path::to::module;`.
    ///
    /// The declaration is at most once and must precede imports and items. A
    /// malformed path still produces a node so the file keeps a stable shape
    /// for the build graph.
    pub(crate) fn parse_module_decl(&mut self) -> ItemId {
        let docs = self.pending_module_docs.take();
        let modifiers = std::mem::take(&mut self.item_modifiers);
        let start = self.current_span();
        self.bump();

        let mut path = Vec::new();
        let mut first_span = None;
        let mut last_span = start.cover(start);
        if let TokenKind::Ident(name) = self.current_kind() {
            let span = self.bump().span;
            path.push(name);
            first_span = Some(span);
            last_span = span;
        } else {
            self.report_module_path_error();
        }

        while self.eat(TokenKind::ColonColon) {
            if let TokenKind::Ident(name) = self.current_kind() {
                let span = self.bump().span;
                path.push(name);
                last_span = span;
            } else {
                self.report_module_path_error();
                break;
            }
        }

        let path_span = first_span.map_or(start, |first| first.cover(last_span));
        if path
            .first()
            .is_some_and(|name| self.interner.resolve(*name) == "root")
        {
            self.error(
                codes::E2075,
                path_span,
                "`root` is reserved and cannot name a module",
            )
            .help("name the module without `root`; `root` refers to the package root in imports")
            .emit();
        }

        if !self.eat(TokenKind::Semicolon) {
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1002,
                self.current_span(),
                format!("expected `;` after the module path, found {found}"),
            )
            .help("write `module path::to::module;`")
            .emit();
            self.skip_to_semicolon_or_item();
        }

        if self.module_decl.is_some() {
            self.error(
                codes::E2069,
                start,
                "a file declares its module path at most once",
            )
            .help("remove the duplicate `module` declaration")
            .emit();
        } else if self.saw_any {
            self.error(
                codes::E2069,
                start,
                "the module declaration must be the first item in the file",
            )
            .help("move `module ...;` above every import and item")
            .emit();
        }
        self.module_decl.get_or_insert(start);
        self.saw_any = true;

        let span = start.cover(self.previous_span());
        let item = self.builder.push_module_item(
            span,
            ModuleDecl {
                path,
                path_span,
                docs: None,
            },
            modifiers,
        );
        self.builder.set_module_docs(item, docs);
        item
    }

    /// Parses `use path;`, including groups and aliases.
    ///
    /// Imports accept only `cfg` before them. Visibility and `test` are rejected
    /// with one diagnostic and do not leave the parser stuck.
    pub(crate) fn parse_use(&mut self, mut modifiers: Modifiers) -> ItemId {
        let start = self.current_span();
        self.bump();

        if modifiers.has_visibility {
            self.error(
                codes::E2079,
                start,
                "an import cannot have a visibility modifier",
            )
            .help("remove `public`, `internal`, or `private`; imports are private to the file")
            .emit();
            modifiers.visibility = Visibility::Private;
            modifiers.has_visibility = false;
        }
        if modifiers.is_test {
            self.error(codes::E2079, start, "an import cannot be a test")
                .help("remove `test`; it applies to functions")
                .emit();
            modifiers.is_test = false;
        }
        if modifiers.export_span.is_some() {
            self.error(codes::E2079, start, "an import cannot be exported")
                .help("remove `export`; only a function can be exported")
                .emit();
            modifiers.export = None;
            modifiers.export_span = None;
        }

        let (root, root_span) = match self.current_kind() {
            TokenKind::Ident(name) => {
                let span = self.bump().span;
                if self.interner.resolve(name) == "root" {
                    (UseRoot::Root, span)
                } else {
                    (UseRoot::Name(name), span)
                }
            }
            _ => {
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E1005,
                    self.current_span(),
                    format!("expected a module name after `use`, found {found}"),
                )
                .help("write an absolute path such as `use root::Type;`")
                .emit();
                (UseRoot::Root, self.current_span())
            }
        };

        let mut segments = Vec::new();
        let mut alias = None;
        let mut alias_span = None;
        let mut aliased = false;
        let mut incomplete_group = false;
        let mut path_end = root_span;

        while self.at(TokenKind::ColonColon) {
            self.bump();
            if self.eat(TokenKind::Star) {
                self.error(
                    codes::E2076,
                    self.previous_span(),
                    "glob imports are not supported",
                )
                .help("import each name explicitly, for example `use a::b::{C, D};`")
                .emit();
                break;
            }
            if self.at(TokenKind::LBrace) {
                let (group, closed) = self.parse_use_group();
                path_end = match &group {
                    UseSegment::Group { span, .. } => *span,
                    UseSegment::Name { span, .. } => *span,
                };
                segments.push(group);
                incomplete_group = !closed;
                break;
            }
            let (name, name_span) = self.expect_ident("a name after `::`");
            if name == Symbol::EMPTY {
                break;
            }
            self.reject_reserved_import_name(name, name_span);
            let mut item_span = name_span;
            let mut item_alias = None;
            let mut item_alias_span = None;
            if self.eat(TokenKind::As) {
                let (name, span) = self.expect_ident("an alias after `as`");
                if name != Symbol::EMPTY {
                    item_alias = Some(name);
                    item_alias_span = Some(span);
                    item_span = item_span.cover(span);
                }
                aliased = true;
            }
            path_end = item_span;
            segments.push(UseSegment::Name {
                name,
                span: item_span,
            });
            if aliased {
                if item_alias.is_some() {
                    alias = item_alias;
                    alias_span = item_alias_span;
                    path_end = item_alias_span.unwrap_or(item_span);
                }
                break;
            }
        }

        if !aliased && self.eat(TokenKind::As) {
            let (name, span) = self.expect_ident("an alias after `as`");
            if name != Symbol::EMPTY {
                alias = Some(name);
                alias_span = Some(span);
                path_end = span;
            }
        }

        if !self.eat(TokenKind::Semicolon) && !incomplete_group {
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1002,
                self.current_span(),
                format!("expected `;` after the import, found {found}"),
            )
            .help("write `use path::to::name;`")
            .emit();
            self.skip_to_semicolon_or_item();
        }

        let path_span = root_span.cover(path_end);
        let span = start.cover(self.previous_span());
        self.saw_any = true;
        self.builder.push_use_item(
            span,
            UseDecl {
                path: UsePath {
                    root,
                    root_span,
                    segments,
                    alias,
                    alias_span,
                    span: path_span,
                },
            },
            modifiers,
        )
    }

    fn parse_use_group(&mut self) -> (UseSegment, bool) {
        let start = self.current_span();
        self.bump();

        let mut items = Vec::new();
        while !self.is_eof() && !self.at(TokenKind::RBrace) && !self.at_group_boundary() {
            if !matches!(self.current_kind(), TokenKind::Ident(_)) {
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E1005,
                    self.current_span(),
                    format!("expected a name in the import group, found {found}"),
                )
                .help("write names separated by commas, as in `{C, D as E}`")
                .emit();
                if !self.at(TokenKind::Comma) && !self.at(TokenKind::RBrace) {
                    self.bump();
                }
                self.eat(TokenKind::Comma);
                continue;
            }

            let (name, name_span) = self.expect_ident("an imported name");
            self.reject_reserved_import_name(name, name_span);
            let mut alias = None;
            let mut alias_span = None;
            let mut span = name_span;
            if self.eat(TokenKind::As) {
                let (name, name_span) = self.expect_ident("an alias after `as`");
                if name != Symbol::EMPTY {
                    alias = Some(name);
                    alias_span = Some(name_span);
                    span = span.cover(name_span);
                }
            }
            items.push(UseItem {
                name,
                name_span,
                alias,
                alias_span,
                span,
            });

            if !self.eat(TokenKind::Comma) && !self.at(TokenKind::RBrace) {
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E1002,
                    self.current_span(),
                    format!("expected `,` between imported names, found {found}"),
                )
                .emit();
                while !self.is_eof() && !self.at(TokenKind::Comma) && !self.at(TokenKind::RBrace) {
                    self.bump();
                }
                self.eat(TokenKind::Comma);
            }
        }

        let closed = self.eat(TokenKind::RBrace);
        let span = start.cover(self.previous_span());
        if items.is_empty() && closed {
            self.error(codes::E1002, span, "an import group cannot be empty")
                .help("list at least one name, or import the module itself")
                .emit();
        }
        if !closed {
            let found = self.describe(self.current_kind());
            self.error(
                codes::E1002,
                self.current_span(),
                format!("expected `}}` to close the import group, found {found}"),
            )
            .help("finish the import before the next item")
            .emit();
            if !self.at_group_boundary() {
                self.skip_to_semicolon_or_item();
            }
        }
        (UseSegment::Group { items, span }, closed)
    }

    fn report_module_path_error(&mut self) {
        let found = self.describe(self.current_kind());
        self.error(
            codes::E2069,
            self.current_span(),
            format!("expected a module name, found {found}"),
        )
        .help("write a path of identifiers separated by `::`")
        .emit();
    }

    fn reject_reserved_import_name(&mut self, name: Symbol, span: Span) {
        if self.interner.resolve(name) == "root" {
            self.error(
                codes::E2075,
                span,
                "`root` is reserved and cannot be an import segment",
            )
            .help("remove `root` from the middle of the path; it is only allowed first")
            .emit();
        }
    }

    fn skip_to_semicolon_or_item(&mut self) {
        while !self.is_eof() && !self.at(TokenKind::Semicolon) && !self.at_item_boundary() {
            self.bump();
        }
        self.eat(TokenKind::Semicolon);
    }

    fn at_group_boundary(&self) -> bool {
        matches!(
            self.current_kind(),
            TokenKind::Func
                | TokenKind::Unsafe
                | TokenKind::Const
                | TokenKind::Static
                | TokenKind::Struct
                | TokenKind::Enum
                | TokenKind::Impl
                | TokenKind::Trait
        ) || self.at_name("extern")
            || self.at_name("use")
            || self.at_name("module")
    }

    fn at_item_boundary(&self) -> bool {
        matches!(
            self.current_kind(),
            TokenKind::Func
                | TokenKind::Unsafe
                | TokenKind::Const
                | TokenKind::Static
                | TokenKind::Struct
                | TokenKind::Enum
                | TokenKind::Impl
                | TokenKind::Trait
        ) || self.at_name("extern")
            || self.at_name("use")
            || self.at_name("module")
            || self.at_name("public")
            || self.at_name("internal")
            || self.at_name("private")
            || self.at_name("cfg")
            || self.at_name("test")
    }
}
