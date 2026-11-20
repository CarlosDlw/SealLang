//! Strings, characters, and escape sequences.

use crate::tests::support::Fixture;
use crate::{TokenKind, lex};

fn string_value(source: &str) -> String {
    let fixture = Fixture::new(source);
    let lexed = fixture.lex();
    fixture.assert_clean();
    match lexed.tokens[0].kind {
        TokenKind::Str(symbol) => fixture.symbol(symbol),
        other => panic!("expected a string for {source:?}, got {other:?}"),
    }
}

fn c_string_value(source: &str) -> String {
    let fixture = Fixture::new(source);
    let lexed = fixture.lex();
    fixture.assert_clean();
    match lexed.tokens[0].kind {
        TokenKind::CStr(symbol) => fixture.symbol(symbol),
        other => panic!("expected a C string for {source:?}, got {other:?}"),
    }
}

fn char_value(source: &str) -> char {
    let fixture = Fixture::new(source);
    let lexed = fixture.lex();
    fixture.assert_clean();
    match lexed.tokens[0].kind {
        TokenKind::Char(character) => character,
        other => panic!("expected a character for {source:?}, got {other:?}"),
    }
}

fn expect_error(source: &str, code: &str, fragment: &str) {
    let fixture = Fixture::new(source);
    let lexed = fixture.lex();
    fixture.expect_single(code, fragment);
    assert!(
        matches!(lexed.tokens[0].kind, TokenKind::Error),
        "malformed literal {source:?} must produce an Error token, got {:?}",
        lexed.tokens[0].kind
    );
}

#[test]
fn strings_cook_their_contents() {
    assert_eq!(string_value("\"abc\""), "abc");
    assert_eq!(string_value("\"\""), "");
    assert_eq!(string_value("\"a\\nb\""), "a\nb");
    assert_eq!(string_value("\"\\t\\r\\0\""), "\t\r\0");
    assert_eq!(string_value("\"\\\\ \\\" \\'\""), "\\ \" '");
    assert_eq!(string_value("\"\\x41\\x7F\""), "A\u{7f}");
    assert_eq!(string_value("\"\\u{41}\\u{1F600}\""), "A\u{1F600}");
    assert_eq!(string_value("\"é日\""), "é日");
}

#[test]
fn c_strings_cook_like_strings_without_the_terminator() {
    assert_eq!(c_string_value("c\"abc\""), "abc");
    assert_eq!(c_string_value("c\"\""), "");
    assert_eq!(c_string_value("c\"a\\nb\\t\\x7F\""), "a\nb\t\u{7f}");
    assert_eq!(c_string_value("c\"é日\""), "é日");
}

#[test]
fn c_strings_reject_an_interior_nul() {
    expect_error("c\"a\\0b\"", "E3090", "NUL");
    expect_error("c\"\\0\"", "E3090", "NUL");
}

#[test]
fn unterminated_c_strings_report_e0006() {
    expect_error("c\"abc", "E0006", "not terminated");
}

#[test]
fn c_string_tokens_keep_their_own_kind() {
    let fixture = Fixture::new("c\"same\" \"same\"");
    let lexed = fixture.lex();
    fixture.assert_clean();
    assert!(matches!(lexed.tokens[0].kind, TokenKind::CStr(_)));
    assert!(matches!(lexed.tokens[1].kind, TokenKind::Str(_)));
}

#[test]
fn strings_accept_raw_tabs() {
    assert_eq!(string_value("\"a\tb\""), "a\tb");
}

#[test]
fn line_continuations_join_lines() {
    assert_eq!(string_value("\"a\\\n   b\""), "ab");
    assert_eq!(string_value("\"a\\\r\n \t b\""), "ab");
    assert_eq!(string_value("\"a\\\n\n\n b\""), "ab");
}

#[test]
fn unterminated_strings_report_e0006() {
    expect_error("\"abc", "E0006", "not terminated");
    expect_error("\"abc\n", "E0006", "line break");
}

#[test]
fn bad_escapes_report_e0005() {
    expect_error("\"\\q\"", "E0005", "unknown escape");
    expect_error("\"\\x4\"", "E0005", "exactly two");
    expect_error("\"\\x80\"", "E0005", "0x7F");
    expect_error("\"\\u1234\"", "E0005", "braces");
    expect_error("\"\\u{}\"", "E0005", "at least one");
    expect_error("\"\\u{1234567}\"", "E0005", "six");
    expect_error(
        "\"\\u{110000}\"",
        "E0005",
        "not a valid Unicode scalar value",
    );
    expect_error("\"\\u{D800}\"", "E0005", "not a valid Unicode scalar value");
    expect_error("\"\\u{41\"", "E0005", "not closed");
}

#[test]
fn raw_control_characters_report_e0013() {
    expect_error("\"\u{0}\"", "E0013", "raw control character");
    expect_error("\"\u{7F}\"", "E0013", "raw control character");
}

#[test]
fn bidi_characters_in_strings_report_e0012() {
    expect_error("\"a\u{202E}b\"", "E0012", "bidirectional");
}

#[test]
fn character_literals_cover_unicode_scalars() {
    assert_eq!(char_value("'a'"), 'a');
    assert_eq!(char_value("'é'"), 'é');
    assert_eq!(char_value("'🦀'"), '🦀');
    assert_eq!(char_value("'\\n'"), '\n');
    assert_eq!(char_value("'\\''"), '\'');
    assert_eq!(char_value("'\\x7F'"), '\u{7f}');
    assert_eq!(char_value("'\\u{1F980}'"), '🦀');
    assert_eq!(char_value("'\t'"), '\t');
}

#[test]
fn malformed_characters_report_e0010() {
    expect_error("''", "E0010", "empty");
    expect_error("'ab'", "E0010", "exactly one");
    expect_error("'a", "E0010", "not terminated");
    expect_error("'a\n", "E0010", "not terminated");
}

#[test]
fn bad_character_escapes_report_e0005() {
    expect_error("'\\q'", "E0005", "unknown escape");
    expect_error("'\\x80'", "E0005", "0x7F");
    expect_error("'\\\n'", "E0005", "line continuation");
}

#[test]
fn bidi_characters_in_character_literals_report_e0012() {
    expect_error("'\u{202E}'", "E0012", "bidirectional");
}

#[test]
fn string_tokens_are_interned() {
    let fixture = Fixture::new("\"same\" \"same\"");
    let lexed = fixture.lex();
    fixture.assert_clean();
    let first = match lexed.tokens[0].kind {
        TokenKind::Str(symbol) => symbol,
        other => panic!("expected string, got {other:?}"),
    };
    let second = match lexed.tokens[1].kind {
        TokenKind::Str(symbol) => symbol,
        other => panic!("expected string, got {other:?}"),
    };
    assert_eq!(first, second);
    assert_eq!(fixture.symbol(first), "same");
}

#[test]
fn lexing_a_string_does_not_swallow_the_next_token() {
    let fixture = Fixture::new("\"a\"+\"b\"");
    let lexed = fixture.lex();
    fixture.assert_clean();
    assert_eq!(fixture.token_texts(&lexed), ["\"a\"", "+", "\"b\""]);
    let _ = lex(&fixture.file, &fixture.dcx, &fixture.interner);
}
