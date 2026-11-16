//! Operator maximal munch, keywords, identifiers, and newline flags.

use crate::tests::support::Fixture;
use crate::{TokenKind, lex};

fn kinds(source: &str) -> Vec<TokenKind> {
    let fixture = Fixture::new(source);
    let lexed = fixture.lex();
    fixture.assert_clean();
    fixture.kinds(&lexed)
}

#[test]
fn every_operator_has_a_token() {
    let cases = [
        ("+", TokenKind::Plus),
        ("-", TokenKind::Minus),
        ("*", TokenKind::Star),
        ("/", TokenKind::Slash),
        ("%", TokenKind::Percent),
        ("++", TokenKind::PlusPlus),
        ("--", TokenKind::MinusMinus),
        ("=", TokenKind::Eq),
        ("==", TokenKind::EqEq),
        ("!=", TokenKind::NotEq),
        ("<", TokenKind::Lt),
        (">", TokenKind::Gt),
        ("<=", TokenKind::LtEq),
        (">=", TokenKind::GtEq),
        ("&&", TokenKind::AndAnd),
        ("||", TokenKind::OrOr),
        ("!", TokenKind::Bang),
        ("&", TokenKind::Amp),
        ("|", TokenKind::Pipe),
        ("^", TokenKind::Caret),
        ("~", TokenKind::Tilde),
        ("<<", TokenKind::Shl),
        (">>", TokenKind::Shr),
        ("+=", TokenKind::PlusEq),
        ("-=", TokenKind::MinusEq),
        ("*=", TokenKind::StarEq),
        ("/=", TokenKind::SlashEq),
        ("%=", TokenKind::PercentEq),
        ("&=", TokenKind::AmpEq),
        ("|=", TokenKind::PipeEq),
        ("^=", TokenKind::CaretEq),
        ("<<=", TokenKind::ShlEq),
        (">>=", TokenKind::ShrEq),
        ("->", TokenKind::Arrow),
        ("(", TokenKind::LParen),
        (")", TokenKind::RParen),
        ("{", TokenKind::LBrace),
        ("}", TokenKind::RBrace),
        ("[", TokenKind::LBracket),
        ("]", TokenKind::RBracket),
        (",", TokenKind::Comma),
        (";", TokenKind::Semicolon),
        (":", TokenKind::Colon),
        (".", TokenKind::Dot),
    ];
    for (source, expected) in cases {
        let kinds = kinds(source);
        assert_eq!(kinds[0], expected, "token for {source:?}");
        assert_eq!(kinds[1], TokenKind::Eof);
    }
}

#[test]
fn maximal_munch_prefers_the_longest_operator() {
    let cases = [
        (">>=", TokenKind::ShrEq),
        ("<<=", TokenKind::ShlEq),
        (">>", TokenKind::Shr),
        (">=", TokenKind::GtEq),
        ("&&", TokenKind::AndAnd),
        ("&=", TokenKind::AmpEq),
        ("--", TokenKind::MinusMinus),
        ("->", TokenKind::Arrow),
        ("++", TokenKind::PlusPlus),
        ("+=", TokenKind::PlusEq),
    ];
    for (source, expected) in cases {
        let kinds = kinds(source);
        assert_eq!(kinds, [expected, TokenKind::Eof], "token for {source:?}");
    }
}

#[test]
fn ambiguous_runs_split_by_maximal_munch() {
    let fixture = Fixture::new("a+++b");
    let lexed = fixture.lex();
    fixture.assert_clean();
    let texts = fixture.token_texts(&lexed);
    assert_eq!(texts, ["a", "++", "+", "b"]);

    let fixture = Fixture::new("a---b");
    let lexed = fixture.lex();
    fixture.assert_clean();
    let texts = fixture.token_texts(&lexed);
    assert_eq!(texts, ["a", "--", "-", "b"]);

    let fixture = Fixture::new("a>>>b");
    let lexed = fixture.lex();
    fixture.assert_clean();
    let texts = fixture.token_texts(&lexed);
    assert_eq!(texts, ["a", ">>", ">", "b"]);
}

#[test]
fn keywords_are_not_identifiers() {
    let fixture = Fixture::new(
        "func funcx struct structx if iffy else elsex let mut mutx while whilex break breakx continue continuex _return return true false",
    );
    let lexed = fixture.lex();
    fixture.assert_clean();
    assert_eq!(
        fixture.kinds(&lexed),
        [
            TokenKind::Func,
            TokenKind::Ident(fixture.interner.intern("funcx")),
            TokenKind::Struct,
            TokenKind::Ident(fixture.interner.intern("structx")),
            TokenKind::If,
            TokenKind::Ident(fixture.interner.intern("iffy")),
            TokenKind::Else,
            TokenKind::Ident(fixture.interner.intern("elsex")),
            TokenKind::Let,
            TokenKind::Mut,
            TokenKind::Ident(fixture.interner.intern("mutx")),
            TokenKind::While,
            TokenKind::Ident(fixture.interner.intern("whilex")),
            TokenKind::Break,
            TokenKind::Ident(fixture.interner.intern("breakx")),
            TokenKind::Continue,
            TokenKind::Ident(fixture.interner.intern("continuex")),
            TokenKind::Ident(fixture.interner.intern("_return")),
            TokenKind::Return,
            TokenKind::True,
            TokenKind::False,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn unicode_identifiers_are_accepted() {
    let fixture = Fixture::new("ação _日本 e\u{301}");
    let lexed = fixture.lex();
    fixture.assert_clean();
    let texts = fixture.token_texts(&lexed);
    assert_eq!(texts, ["ação", "_日本", "e\u{301}"]);
}

#[test]
fn underscores_are_identifiers() {
    let fixture = Fixture::new("_ _x");
    let lexed = fixture.lex();
    fixture.assert_clean();
    assert_eq!(fixture.token_texts(&lexed), ["_", "_x"]);
}

#[test]
fn newline_flags_are_per_token() {
    let fixture = Fixture::new("func\nmain");
    let lexed = fixture.lex();
    fixture.assert_clean();
    assert!(!lexed.tokens[0].preceded_by_newline);
    assert!(lexed.tokens[1].preceded_by_newline);
    assert!(!lexed.tokens[2].preceded_by_newline);

    let fixture = Fixture::new("func main\n");
    let lexed = fixture.lex();
    let eof = lexed.tokens.last().expect("eof exists");
    assert!(eof.preceded_by_newline);
    assert_eq!(eof.kind, TokenKind::Eof);
}

#[test]
fn spans_are_exact_and_ordered() {
    let fixture = Fixture::new("let x = 42;\n");
    let lexed = fixture.lex();
    fixture.assert_clean();
    assert_eq!(fixture.token_texts(&lexed), ["let", "x", "=", "42", ";"]);
    assert_eq!(lexed.tokens.last().expect("eof").span.len(), 0);
}

#[test]
fn repeated_lexing_is_deterministic() {
    let fixture = Fixture::new("func main() -> i32 { 1 + 2 }");
    let first = fixture.lex();
    let second = fixture.lex();
    assert_eq!(first, second);
    let third = lex(&fixture.file, &fixture.dcx, &fixture.interner);
    assert_eq!(first, third);
}

#[test]
fn empty_source_has_only_eof() {
    let fixture = Fixture::new("");
    let lexed = fixture.lex();
    fixture.assert_clean();
    assert_eq!(lexed.tokens.len(), 1);
    assert_eq!(lexed.tokens[0].kind, TokenKind::Eof);
    assert!(lexed.trivia.is_empty());
}
