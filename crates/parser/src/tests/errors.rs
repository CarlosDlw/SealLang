//! Diagnostic codes and targeted recovery.

use seal_ast::ItemKind;

use crate::tests::support::Fixture;

#[test]
fn recovery_keeps_the_function_after_a_broken_one() {
    let fixture = Fixture::new("func broken( { let x = ; } func good() -> i32 { 1 }");
    let ast = fixture.parse();
    let codes = fixture.codes();
    assert!(
        codes.iter().any(|code| code == "E1002" || code == "E1003"),
        "expected syntax errors, got {codes:?}"
    );
    assert_eq!(ast.program().len(), 2);
    assert!(matches!(
        ast.item(ast.program().items()[1]).kind,
        ItemKind::Func(_)
    ));
}

#[test]
fn top_level_garbage_is_skipped_until_the_next_function() {
    let fixture = Fixture::new("let x = 1; @ nope; func main() {}");
    let ast = fixture.parse();
    fixture.expect("E1001", "expected an item");
    assert_eq!(ast.program().len(), 2);
    assert!(matches!(
        ast.item(ast.program().items()[1]).kind,
        ItemKind::Func(_)
    ));
}

#[test]
fn a_bad_expression_does_not_lose_following_statements() {
    let fixture = Fixture::new("func main() { let x = ; let y = 2; y }");
    let ast = fixture.parse();
    fixture.expect("E1003", "expected an expression");
    let declaration = crate::tests::support::sole_function(&ast);
    let block = crate::tests::support::body(&ast, declaration);
    assert_eq!(block.statements.len(), 2);
    assert!(block.tail.is_some());
}

#[test]
fn unexpected_keywords_in_expression_position_are_diagnosed() {
    let fixture = Fixture::new("func main() { func }");
    let _ = fixture.parse();
    fixture.expect("E1003", "expected an expression");
}

#[test]
fn a_trailing_token_after_a_function_is_diagnosed() {
    let fixture = Fixture::new("func main() {} strange");
    let ast = fixture.parse();
    fixture.expect("E1001", "expected an item");
    assert_eq!(ast.program().len(), 2);
}

#[test]
fn every_parser_error_has_a_stable_code() {
    let cases = [
        ("let x = 1;", "E1001"),
        ("func f()", "E1002"),
        ("func main() { ; }", "E1003"),
        ("func f(a:) {}", "E1004"),
        ("func () {}", "E1005"),
        ("func main() { 1 = 2 }", "E1006"),
        ("func main() { let x; }", "E1007"),
    ];
    for (source, expected) in cases {
        let fixture = Fixture::new(source);
        let _ = fixture.parse();
        assert!(
            fixture.codes().iter().any(|code| code == expected),
            "source {source:?} did not produce {expected}: {:?}",
            fixture.codes()
        );
    }
}

#[test]
fn deeply_nested_parentheses_hit_the_nesting_limit() {
    let source = format!("func main() {{ {}1{} }}", "(".repeat(400), ")".repeat(400));
    let fixture = Fixture::new(&source);
    let _ = fixture.parse();
    fixture.expect("E1008", "nested too deeply");
}

#[test]
fn deeply_nested_pointer_types_hit_the_nesting_limit() {
    let source = format!("func f(p: {}i32) {{}}", "*".repeat(400));
    let fixture = Fixture::new(&source);
    let _ = fixture.parse();
    fixture.expect("E1008", "nested too deeply");
}

#[test]
fn diagnostics_are_not_duplicated_for_one_mistake() {
    let fixture = Fixture::new("func main() { let x = ; }");
    let _ = fixture.parse();
    let e1003 = fixture
        .codes()
        .iter()
        .filter(|code| code.as_str() == "E1003")
        .count();
    assert_eq!(e1003, 1);
}
