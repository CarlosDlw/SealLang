//! Type syntax.

use seal_ast::{PrimitiveType, TypeId, TypeKind};
use seal_error_codes as codes;
use seal_lexer::TokenKind;

use crate::parser::{MAX_DEPTH, Parser};

impl Parser<'_> {
    /// Parses a type: a primitive name or a reference to another type.
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
        } else if self.eat(TokenKind::LBracket) {
            let element = self.parse_type();
            let _ = self.expect(TokenKind::Semicolon, "`;` after the array element type");
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
                    // namespace is known; the parser only records the name.
                    let token = self.bump();
                    self.builder.push_type(TypeKind::Path(symbol), token.span)
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
}

/// Delimiters after which a missing type should not force token consumption:
/// the initializer or the surrounding syntax still needs them.
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
    )
}
