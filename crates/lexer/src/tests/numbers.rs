//! Numeric literal grammar, values, separators, suffixes, and overflow.

use crate::tests::support::Fixture;
use crate::{NumericBase, NumericSuffix, TokenKind};

fn integer(source: &str) -> (u128, NumericBase, Option<NumericSuffix>) {
    let fixture = Fixture::new(source);
    let lexed = fixture.lex();
    fixture.assert_clean();
    match lexed.tokens[0].kind {
        TokenKind::Int {
            value,
            base,
            suffix,
        } => (value, base, suffix),
        other => panic!("expected an integer for {source:?}, got {other:?}"),
    }
}

fn float(source: &str) -> (f64, Option<NumericSuffix>) {
    let fixture = Fixture::new(source);
    let lexed = fixture.lex();
    fixture.assert_clean();
    match lexed.tokens[0].kind {
        TokenKind::Float { value, suffix } => (value, suffix),
        other => panic!("expected a float for {source:?}, got {other:?}"),
    }
}

fn expect_error(source: &str, code: &str, message_fragment: &str) {
    let fixture = Fixture::new(source);
    let lexed = fixture.lex();
    fixture.expect_single(code, message_fragment);
    assert!(
        matches!(lexed.tokens[0].kind, TokenKind::Error),
        "malformed literal {source:?} must produce an Error token, got {:?}",
        lexed.tokens[0].kind
    );
}

#[test]
fn integers_use_every_base() {
    let cases = [
        ("0", 0, NumericBase::Decimal),
        ("42", 42, NumericBase::Decimal),
        ("1_000_000", 1_000_000, NumericBase::Decimal),
        ("1__2", 12, NumericBase::Decimal),
        ("00", 0, NumericBase::Decimal),
        ("0b1010", 10, NumericBase::Binary),
        ("0b1111_0000", 240, NumericBase::Binary),
        ("0o755", 493, NumericBase::Octal),
        ("0o7_5_5", 493, NumericBase::Octal),
        ("0xFF", 255, NumericBase::Hexadecimal),
        ("0xFF_FF", 65_535, NumericBase::Hexadecimal),
        ("0xdead_BEEF", 0xDEAD_BEEF, NumericBase::Hexadecimal),
    ];
    for (source, value, base) in cases {
        let (actual_value, actual_base, suffix) = integer(source);
        assert_eq!(actual_value, value, "value of {source:?}");
        assert_eq!(actual_base, base, "base of {source:?}");
        assert_eq!(suffix, None, "suffix of {source:?}");
    }
}

#[test]
fn integers_accept_the_widest_value() {
    let maximum = u128::MAX.to_string();
    let (value, base, suffix) = integer(&maximum);
    assert_eq!(value, u128::MAX);
    assert_eq!(base, NumericBase::Decimal);
    assert_eq!(suffix, None);
}

#[test]
fn integer_suffixes_are_parsed() {
    let cases = [
        ("1u8", NumericSuffix::U8),
        ("1u16", NumericSuffix::U16),
        ("1u32", NumericSuffix::U32),
        ("1u64", NumericSuffix::U64),
        ("1u128", NumericSuffix::U128),
        ("1usize", NumericSuffix::Usize),
        ("1i8", NumericSuffix::I8),
        ("1i16", NumericSuffix::I16),
        ("1i32", NumericSuffix::I32),
        ("1i64", NumericSuffix::I64),
        ("1i128", NumericSuffix::I128),
        ("1isize", NumericSuffix::Isize),
        ("0xFFu8", NumericSuffix::U8),
    ];
    for (source, expected) in cases {
        let (_, _, suffix) = integer(source);
        assert_eq!(suffix, Some(expected), "suffix of {source:?}");
    }
}

#[test]
fn signed_suffix_range_is_deferred() {
    // `-128i8` is valid, so the lexer cannot reject `128i8` by itself.
    let (value, _, suffix) = integer("128i8");
    assert_eq!(value, 128);
    assert_eq!(suffix, Some(NumericSuffix::I8));
}

#[test]
fn floats_use_every_decimal_form() {
    let cases = [
        ("3.43", 3.43, None),
        ("3.25", 3.25, None),
        ("1e3", 1000.0, None),
        ("1E+3", 1000.0, None),
        ("1.5e-3", 0.0015, None),
        ("1.", 1.0, None),
        ("1_000.000_1", 1000.0001, None),
        ("1f32", 1.0, Some(NumericSuffix::F32)),
        ("1.0f64", 1.0, Some(NumericSuffix::F64)),
        ("1e3f32", 1000.0, Some(NumericSuffix::F32)),
    ];
    for (source, expected, expected_suffix) in cases {
        let (value, suffix) = float(source);
        assert_eq!(value, expected, "value of {source:?}");
        assert_eq!(suffix, expected_suffix, "suffix of {source:?}");
    }
}

#[test]
fn a_range_after_an_integer_is_one_token() {
    let fixture = Fixture::new("1..2");
    let lexed = fixture.lex();
    fixture.assert_clean();
    assert_eq!(
        fixture.kinds(&lexed),
        [
            TokenKind::Int {
                value: 1,
                base: NumericBase::Decimal,
                suffix: None
            },
            TokenKind::DotDot,
            TokenKind::Int {
                value: 2,
                base: NumericBase::Decimal,
                suffix: None
            },
            TokenKind::Eof,
        ]
    );
}

#[test]
fn method_calls_after_integers_are_not_floats() {
    let fixture = Fixture::new("1.foo");
    let lexed = fixture.lex();
    fixture.assert_clean();
    let kinds = fixture.kinds(&lexed);
    assert_eq!(
        kinds[0],
        TokenKind::Int {
            value: 1,
            base: NumericBase::Decimal,
            suffix: None
        }
    );
    assert_eq!(kinds[1], TokenKind::Dot);
    assert!(matches!(kinds[2], TokenKind::Ident(_)));
    assert_eq!(
        fixture.symbol(match kinds[2] {
            TokenKind::Ident(s) => s,
            _ => unreachable!(),
        }),
        "foo"
    );
}

#[test]
fn malformed_integers_report_e0007() {
    let cases = [
        ("1_", "separator"),
        ("0x_FF", "separator"),
        ("0b102", "not a valid digit"),
        ("0o8", "not a valid digit"),
        ("0x", "no digits"),
        ("0b", "no digits"),
        ("0XFF", "lowercase"),
        ("0b1.0", "fractional"),
    ];
    for (source, fragment) in cases {
        expect_error(source, "E0007", fragment);
    }
}

#[test]
fn malformed_exponents_report_e0007() {
    let cases = [
        ("1e", "exponent"),
        ("1E", "exponent"),
        ("1e+", "exponent"),
        ("1e-", "exponent"),
        ("1e_3", "separator"),
        ("1e3_", "separator"),
    ];
    for (source, fragment) in cases {
        expect_error(source, "E0007", fragment);
    }
}

#[test]
fn invalid_suffixes_report_e0008() {
    let cases = [
        ("123abc", "not a valid numeric suffix"),
        ("1u8f32", "not a valid numeric suffix"),
        ("1.0u8", "not valid on a floating-point literal"),
        ("1e3u8", "not valid on a floating-point literal"),
    ];
    for (source, fragment) in cases {
        expect_error(source, "E0008", fragment);
    }
}

#[test]
fn out_of_range_literals_report_e0009() {
    let too_many_digits = "9".repeat(40);
    let too_many_binary_digits = format!("0b{}", "1".repeat(129));
    let too_many_hex_digits = format!("0x{}", "f".repeat(33));
    let cases = [
        (too_many_digits.as_str(), "integer literal is too large"),
        (
            too_many_binary_digits.as_str(),
            "integer literal is too large",
        ),
        (too_many_hex_digits.as_str(), "integer literal is too large"),
        ("1e999", "out of range"),
        ("1e40f32", "out of range for `f32`"),
    ];
    for (source, fragment) in cases {
        expect_error(source, "E0009", fragment);
    }
}

#[test]
fn one_error_per_literal() {
    let fixture = Fixture::new("0b102");
    let _ = fixture.lex();
    assert_eq!(fixture.codes(), ["E0007"]);
}

#[test]
fn lexing_continues_after_a_bad_literal() {
    let fixture = Fixture::new("1_ + 0b2 + 0x + 1e");
    let lexed = fixture.lex();
    assert_eq!(fixture.codes(), ["E0007", "E0007", "E0007", "E0007"]);
    assert_eq!(fixture.kinds(&lexed).last(), Some(&TokenKind::Eof));
    assert_eq!(
        fixture
            .kinds(&lexed)
            .iter()
            .filter(|kind| matches!(kind, TokenKind::Error))
            .count(),
        4
    );
}

#[test]
fn chained_tuple_indices_are_integer_tokens_after_dots() {
    let fixture = Fixture::new("pair.0.1");
    let lexed = fixture.lex();
    fixture.assert_clean();
    let kinds = fixture.kinds(&lexed);
    assert_eq!(kinds.len(), 6);
    assert!(matches!(kinds[0], TokenKind::Ident(_)));
    assert_eq!(kinds[1], TokenKind::Dot);
    assert_eq!(
        kinds[2],
        TokenKind::Int {
            value: 0,
            base: NumericBase::Decimal,
            suffix: None
        }
    );
    assert_eq!(kinds[3], TokenKind::Dot);
    assert_eq!(
        kinds[4],
        TokenKind::Int {
            value: 1,
            base: NumericBase::Decimal,
            suffix: None
        }
    );
    assert_eq!(fixture.token_texts(&lexed), ["pair", ".", "0", ".", "1"]);
}

#[test]
fn leading_dot_is_not_a_float() {
    let fixture = Fixture::new(".5");
    let lexed = fixture.lex();
    fixture.assert_clean();
    assert_eq!(
        fixture.kinds(&lexed),
        [
            TokenKind::Dot,
            TokenKind::Int {
                value: 5,
                base: NumericBase::Decimal,
                suffix: None
            },
            TokenKind::Eof,
        ]
    );
}

#[test]
fn literal_spans_cover_exactly_the_lexeme() {
    let fixture = Fixture::new("let x = 0xFF_u8;");
    let lexed = fixture.lex();
    let texts = fixture.token_texts(&lexed);
    assert_eq!(texts, ["let", "x", "=", "0xFF_u8", ";"]);
    // The separator before a suffix is rejected, and the token keeps the span.
    assert_eq!(fixture.codes(), ["E0007"]);
    assert_eq!(fixture.text(lexed.tokens[3].span), "0xFF_u8");
}
