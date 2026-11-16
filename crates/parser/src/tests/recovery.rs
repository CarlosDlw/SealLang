//! Invariants, determinism, and hostile input.

use seal_ast::ItemKind;

use crate::tests::support::{Fixture, assert_invariants, parse_clean};

#[test]
fn tree_invariants_hold_for_production_like_sources() {
    let sources = [
        "",
        "func main() {}",
        "func main() -> i32 { 0 }",
        "func add(a: i32, b: *f32,) -> i32 { a + b }",
        "func main() { let a = 1; let b: i32; a += b; b++ }",
        "func main() { let x = ; let y = 2; y }",
        "func broken( { let x = ; } func good() -> i32 { 1 }",
        "func main() { (1 + 2) * 3 }",
    ];
    for source in sources {
        let fixture = Fixture::new(source);
        let ast = fixture.parse();
        assert_invariants(&fixture, &ast);
    }
}

#[test]
fn all_error_items_keep_valid_spans() {
    let fixture = Fixture::new("garbage func main() {} more garbage");
    let ast = fixture.parse();
    assert_invariants(&fixture, &ast);
    assert!(
        ast.program()
            .items()
            .iter()
            .any(|id| matches!(ast.item(*id).kind, ItemKind::Error))
    );
}

#[test]
fn parsing_is_deterministic() {
    let (_, first) = parse_clean("func main() { let a = 1; a + 2 }");
    let (_, second) = parse_clean("func main() { let a = 1; a + 2 }");
    assert_eq!(first, second);
}

#[test]
fn arbitrary_ascii_input_never_panics() {
    const ALPHABET: &[u8] = b"abc012_+-*/%<>=!&|^~(){}[],;:.\\\"' \n\r\t@#$`";
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    for _ in 0..150 {
        let length = next(&mut state) as usize % 160;
        let mut bytes = Vec::with_capacity(length);
        for _ in 0..length {
            bytes.push(ALPHABET[next(&mut state) as usize % ALPHABET.len()]);
        }
        let source = String::from_utf8(bytes).expect("alphabet is ASCII");
        let fixture = Fixture::new(&source);
        let ast = fixture.parse();
        assert_invariants(&fixture, &ast);
    }
}

fn next(state: &mut u64) -> u8 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    (*state >> 33) as u8
}
