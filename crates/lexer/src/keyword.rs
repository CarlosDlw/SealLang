//! Keyword recognition.
//!
//! Keywords are exactly the reserved words of the current language surface.
//! Type names such as `i32` stay identifiers; name resolution gives them meaning.
//! Everything else that is not listed here is an identifier, so adding a keyword
//! later is a deliberate change in one place.

use crate::token::TokenKind;

/// Returns the keyword token for `text`, if it is reserved.
pub(crate) fn lookup(text: &str) -> Option<TokenKind> {
    Some(match text {
        "func" => TokenKind::Func,
        "struct" => TokenKind::Struct,
        "enum" => TokenKind::Enum,
        "impl" => TokenKind::Impl,
        "trait" => TokenKind::Trait,
        "for" => TokenKind::For,
        "in" => TokenKind::In,
        "if" => TokenKind::If,
        "else" => TokenKind::Else,
        "let" => TokenKind::Let,
        "mut" => TokenKind::Mut,
        "while" => TokenKind::While,
        "match" => TokenKind::Match,
        "break" => TokenKind::Break,
        "continue" => TokenKind::Continue,
        "return" => TokenKind::Return,
        "true" => TokenKind::True,
        "false" => TokenKind::False,
        _ => return None,
    })
}
