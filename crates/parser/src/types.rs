//! Type syntax.

use seal_ast::{PrimitiveType, TraitPath, TypeId, TypeKind, TypePath};
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
            let mutable = self.eat(TokenKind::Mut);
            let pointee = self.parse_type();
            let span = start.cover(self.builder.ast().ty(pointee).span);
            self.builder
                .push_type(TypeKind::Pointer { pointee, mutable }, span)
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
                    // explicit arguments.
                    let token = self.bump();
                    let args = self.parse_optional_generic_args();
                    let end = args.as_ref().map_or(token.span, |args| args.span);
                    let path = TypePath {
                        name: symbol,
                        name_span: token.span,
                        args,
                    };
                    self.builder
                        .push_type(TypeKind::Path(path), token.span.cover(end))
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

        let path = self.parse_trait_path();
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
    )
}
