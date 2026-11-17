//! Parsing `while let` statements.

use seal_ast::{PatternKind, StmtKind, VariantPatternFields};

use super::support::{Fixture, assert_invariants, body, function, parse_clean};

#[test]
fn while_let_parses_a_pattern_scrutinee_and_body() {
    let (fixture, ast) = parse_clean(
        "enum Option { None, Some(i32) } \
         func main(value: Option) { \
           while let Option::Some(inner) = value { break; } \
         }",
    );
    assert_invariants(&fixture, &ast);

    let declaration = function(&ast, 1);
    let statements = &body(&ast, declaration).statements;
    assert_eq!(statements.len(), 1);
    let StmtKind::WhileLet {
        pattern,
        scrutinee: _,
        body: loop_body,
    } = &ast.stmt(statements[0]).kind
    else {
        panic!("expected a while let statement");
    };
    let PatternKind::Variant { fields, .. } = &ast.pattern(*pattern).kind else {
        panic!("expected a variant pattern");
    };
    assert!(matches!(fields, VariantPatternFields::Positional(_)));
    assert_eq!(ast.block(*loop_body).statements.len(), 1);
}

#[test]
fn a_missing_equals_after_the_while_let_pattern_is_reported() {
    let fixture = Fixture::new("func main() { while let 0 5 { } }");
    let ast = fixture.parse();
    fixture.expect("E1002", "expected `=`");
    assert_invariants(&fixture, &ast);
}
