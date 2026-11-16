//! Comments, doc comments, nesting, and newline tracking.

use crate::tests::support::Fixture;
use crate::{TokenKind, TriviaKind};

#[test]
fn comments_are_trivia_not_tokens() {
    let fixture = Fixture::new("func /* c */ main // tail");
    let lexed = fixture.lex();
    fixture.assert_clean();
    assert_eq!(
        fixture.kinds(&lexed),
        [
            TokenKind::Func,
            TokenKind::Ident(fixture.interner.intern("main")),
            TokenKind::Eof,
        ]
    );
    assert_eq!(lexed.trivia.len(), 2);
    assert_eq!(lexed.trivia[0].kind, TriviaKind::BlockComment);
    assert_eq!(fixture.text(lexed.trivia[0].span), "/* c */");
    assert_eq!(lexed.trivia[1].kind, TriviaKind::LineComment);
    assert_eq!(fixture.text(lexed.trivia[1].span), "// tail");
}

#[test]
fn doc_comment_styles_are_distinguished() {
    let fixture = Fixture::new(
        "/// outer\n//! inner\n/** outer block */\n/*! inner block */\n//// plain\n/**/ plain",
    );
    let lexed = fixture.lex();
    fixture.assert_clean();
    let kinds: Vec<TriviaKind> = lexed.trivia.iter().map(|trivia| trivia.kind).collect();
    assert_eq!(
        kinds,
        [
            TriviaKind::OuterLineDoc,
            TriviaKind::InnerLineDoc,
            TriviaKind::OuterBlockDoc,
            TriviaKind::InnerBlockDoc,
            TriviaKind::LineComment,
            TriviaKind::BlockComment,
        ]
    );
    assert!(kinds.iter().filter(|kind| kind.is_doc()).count() == 4);
}

#[test]
fn block_comments_nest() {
    let fixture = Fixture::new("/* a /* b */ c */ x");
    let lexed = fixture.lex();
    fixture.assert_clean();
    assert_eq!(lexed.trivia.len(), 1);
    assert_eq!(fixture.text(lexed.trivia[0].span), "/* a /* b */ c */");
    let kinds = fixture.kinds(&lexed);
    assert_eq!(kinds.len(), 2);
    assert_eq!(kinds[0], TokenKind::Ident(fixture.interner.intern("x")));
}

#[test]
fn unterminated_block_comments_report_e0011() {
    for source in ["/* open", "/* /* */", "/** doc"] {
        let fixture = Fixture::new(source);
        let lexed = fixture.lex();
        fixture.expect_single("E0011", "unterminated block comment");
        assert_eq!(lexed.trivia.len(), 1);
        assert_eq!(lexed.trivia[0].span.hi(), fixture.file.end());
    }
}

#[test]
fn newlines_inside_block_comments_are_recorded() {
    let fixture = Fixture::new("a /*\n*/ b");
    let lexed = fixture.lex();
    fixture.assert_clean();
    let kinds = fixture.kinds(&lexed);
    assert_eq!(kinds.len(), 3);
    assert!(!lexed.tokens[0].preceded_by_newline);
    assert!(lexed.tokens[1].preceded_by_newline);
}

#[test]
fn line_comments_do_not_hide_the_newline() {
    let fixture = Fixture::new("a // comment\nb");
    let lexed = fixture.lex();
    fixture.assert_clean();
    assert!(!lexed.tokens[0].preceded_by_newline);
    assert!(lexed.tokens[1].preceded_by_newline);
}

#[test]
fn bidi_characters_in_comments_report_e0012() {
    for source in ["// \u{202E}", "/* \u{2066} */"] {
        let fixture = Fixture::new(source);
        let _ = fixture.lex();
        fixture.expect_single("E0012", "bidirectional");
    }
}

#[test]
fn comment_only_sources_produce_just_eof() {
    let fixture = Fixture::new("// a\n/* b */\n");
    let lexed = fixture.lex();
    fixture.assert_clean();
    assert_eq!(lexed.tokens.len(), 1);
    assert_eq!(lexed.tokens[0].kind, TokenKind::Eof);
    assert!(lexed.tokens[0].preceded_by_newline);
    assert_eq!(lexed.trivia.len(), 2);
}
