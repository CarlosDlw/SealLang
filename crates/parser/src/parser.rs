//! Parser state, token access, diagnostics, and top-level items.

use seal_ast::{
    Ast, AstBuilder, ExprId, FnDecl, ImplDecl, ItemId, ItemKind, ParamId, StructDecl, TypeId,
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
            } else if self.at(TokenKind::Impl) {
                let item = self.parse_impl();
                self.items.push(item);
            } else {
                let start = self.current_span();
                let found = self.describe(self.current_kind());
                self.error(
                    codes::E1001,
                    start,
                    format!("expected an item, found {found}"),
                )
                .help("top-level items start with `func`, `struct`, or `impl`")
                .emit();
                while !self.is_eof()
                    && !self.at(TokenKind::Func)
                    && !self.at(TokenKind::Struct)
                    && !self.at(TokenKind::Impl)
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
        let (name, name_span) = self.expect_ident("a function name");
        let _ = self.expect(TokenKind::LParen, "`(` after the function name");

        let mut params: Vec<ParamId> = Vec::new();
        while !self.at(TokenKind::RParen)
            && !self.at(TokenKind::Arrow)
            && !self.at(TokenKind::LBrace)
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
                params,
                ret,
                body,
            }),
            span,
        )
    }

    /// Parses `impl Type { func ... }`.
    ///
    /// Seal has no traits yet, so the target is always a single nominal type
    /// name. Functions are parsed by the regular function parser and kept in
    /// the item arena; the implementation item references them, keeping the
    /// whole tree reachable without a second declaration grammar.
    fn parse_impl(&mut self) -> ItemId {
        let keyword = self.bump();
        let (type_name, type_span) = self.expect_ident("a type name after `impl`");
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
                type_name,
                type_span,
                functions,
            }),
            span,
        )
    }

    fn parse_struct(&mut self) -> ItemId {
        let keyword = self.bump();
        let (name, name_span) = self.expect_ident("a struct name");
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
                fields,
            }),
            span,
        )
    }

    /// Returns the current token; at or past the end, a synthetic `Eof` tied to
    /// the previous token's position.
    pub(crate) fn current(&self) -> Token {
        self.tokens.get(self.index).copied().unwrap_or(Token {
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
        TokenKind::If => "if",
        TokenKind::Else => "else",
        TokenKind::Let => "let",
        TokenKind::Mut => "mut",
        TokenKind::While => "while",
        TokenKind::Break => "break",
        TokenKind::Continue => "continue",
        TokenKind::Return => "return",
        TokenKind::True => "true",
        TokenKind::False => "false",
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
            | TokenKind::Error
    )
}
