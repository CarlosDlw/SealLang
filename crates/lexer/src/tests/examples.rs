//! The language examples must lex without errors.

use std::path::PathBuf;

use crate::tests::support::Fixture;
use crate::{TokenKind, lex};

fn example(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

#[test]
fn all_examples_lex_cleanly() {
    for name in [
        "0001_main.sl",
        "0002_variables.sl",
        "0003_operators.sl",
        "004_conditionals.sl",
        "005_while_loop.sl",
        "007_runtime_panic.sl",
        "008_exclusive_references.sl",
        "010_slices.sl",
        "011_methods.sl",
        "012_user_methods.sl",
        "013_method_borrows.sl",
        "014_method_namespaces.sl",
        "015_traits.sl",
        "016_trait_receivers.sl",
        "017_enums.sl",
        "018_tuple_variants.sl",
        "019_match.sl",
        "020_match_advanced.sl",
        "021_match_slice.sl",
        "022_match_reference.sl",
        "023_for.sl",
        "024_if_let.sl",
        "025_while_let.sl",
        "026_tuples.sl",
    ] {
        let source = example(name);
        let fixture = Fixture::new(&source);
        let lexed = fixture.lex();
        fixture.assert_clean();
        assert!(
            matches!(
                lexed.tokens.first().map(|token| token.kind),
                Some(TokenKind::Func | TokenKind::Struct | TokenKind::Enum)
            ),
            "first token of {name}"
        );
        assert_eq!(
            lexed.tokens.last().map(|token| token.kind),
            Some(TokenKind::Eof),
            "last token of {name}"
        );
        assert!(lexed.tokens.len() > 5, "{name} should have tokens");
        assert!(!lexed.trivia.is_empty(), "{name} has comments");
    }
}

#[test]
fn operator_example_uses_the_compound_operators() {
    let source = example("0003_operators.sl");
    let fixture = Fixture::new(&source);
    let lexed = fixture.lex();
    fixture.assert_clean();
    let kinds: Vec<TokenKind> = lexed.tokens.iter().map(|token| token.kind).collect();
    for expected in [
        TokenKind::PlusPlus,
        TokenKind::MinusMinus,
        TokenKind::ShlEq,
        TokenKind::ShrEq,
        TokenKind::Tilde,
        TokenKind::Arrow,
        TokenKind::PlusEq,
    ] {
        assert!(
            kinds.contains(&expected),
            "expected {expected:?} in 0003_operators.sl"
        );
    }
}

#[test]
fn examples_are_stable_across_relexing() {
    let source = example("0002_variables.sl");
    let fixture = Fixture::new(&source);
    let first = fixture.lex();
    let second = lex(&fixture.file, &fixture.dcx, &fixture.interner);
    assert_eq!(first, second);
}
