//! Error recovery, invariants, and hostile input.

use crate::tests::support::{Fixture, assert_invariants};
use crate::{TokenKind, lex};

#[test]
fn unknown_characters_are_diagnosed_and_skipped() {
    let cases = [
        ("@", "E0004", "not valid Seal source"),
        ("$", "E0004", "not valid Seal source"),
        ("#", "E0004", "not valid Seal source"),
        ("`", "E0004", "not valid Seal source"),
        ("\u{00A0}", "E0004", "whitespace"),
        ("\u{000B}", "E0004", "whitespace"),
        ("a\u{FEFF}b", "E0004", "byte order mark"),
    ];
    for (source, code, fragment) in cases {
        let fixture = Fixture::new(source);
        let lexed = fixture.lex();
        fixture.expect_single(code, fragment);
        assert!(
            lexed
                .tokens
                .iter()
                .any(|token| token.kind == TokenKind::Error)
        );
        assert_eq!(lexed.tokens.last().expect("eof").kind, TokenKind::Eof);
    }
}

#[test]
fn bare_carriage_returns_report_e0004() {
    let fixture = Fixture::new("a\rb");
    let lexed = fixture.lex();
    fixture.expect_single("E0004", "bare carriage return");
    assert_eq!(
        lexed.tokens[0].kind,
        TokenKind::Ident(fixture.interner.intern("a"))
    );
    assert_eq!(
        lexed.tokens[1].kind,
        TokenKind::Ident(fixture.interner.intern("b"))
    );
    assert!(lexed.tokens[1].preceded_by_newline);
}

#[test]
fn bidi_controls_report_e0012() {
    for source in ["\u{202A}", "a\u{202E}b", "\u{2069}"] {
        let fixture = Fixture::new(source);
        let lexed = fixture.lex();
        fixture.expect_single("E0012", "bidirectional");
        assert_eq!(lexed.tokens.last().expect("eof").kind, TokenKind::Eof);
    }
}

#[test]
fn errors_do_not_stop_later_tokens() {
    let fixture = Fixture::new("@ let x = 1; #");
    let lexed = fixture.lex();
    assert_eq!(fixture.codes(), ["E0004", "E0004"]);
    let texts = fixture.token_texts(&lexed);
    assert_eq!(texts, ["@", "let", "x", "=", "1", ";", "#"]);
    assert_eq!(lexed.tokens.last().expect("eof").kind, TokenKind::Eof);
}

#[test]
fn invariants_hold_for_every_production() {
    let sources = [
        "",
        "func main() -> i32 { 0 }",
        "let a = 0b1010 + 0xFFu8 + 1.5e-3;",
        "/* a /* b */ c */ /// doc\n\"x\\n\" 'y'",
        "1_ 0x 0b2 123abc \"open",
        "\u{202E} /* \u{2066} */",
    ];
    for source in sources {
        let fixture = Fixture::new(source);
        let lexed = fixture.lex();
        assert_invariants(&fixture, &lexed);
    }
}

#[test]
fn arbitrary_ascii_input_never_panics() {
    const ALPHABET: &[u8] = b"abcXYZ018_+-*/%<>=!&|^~(){}[],;:.\\\"' \n\r\t@#$`";
    let mut state = 0x1234_5678_9abc_def0u64;
    for _ in 0..200 {
        let length = next(&mut state) as usize % 200;
        let mut bytes = Vec::with_capacity(length);
        for _ in 0..length {
            bytes.push(ALPHABET[next(&mut state) as usize % ALPHABET.len()]);
        }
        let source = String::from_utf8(bytes).expect("alphabet is ASCII");
        let fixture = Fixture::new(&source);
        let lexed = fixture.lex();
        assert_invariants(&fixture, &lexed);
    }
}

#[test]
fn lexing_the_same_file_twice_is_stable() {
    let fixture = Fixture::new("func main() { let s = \"a\\n\"; }");
    let first = fixture.lex();
    let second = lex(&fixture.file, &fixture.dcx, &fixture.interner);
    assert_eq!(first, second);
}

fn next(state: &mut u64) -> u8 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    (*state >> 33) as u8
}
