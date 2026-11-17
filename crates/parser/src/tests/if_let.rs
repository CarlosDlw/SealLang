//! Parsing `if let` expressions.

use seal_ast::{ExprKind, PatternKind, VariantPatternFields};

use super::support::{Fixture, assert_invariants, function, parse_clean, sole_function, tail};

#[test]
fn if_let_parses_a_pattern_and_both_branches() {
    let (fixture, ast) = parse_clean(
        "func main(value: i32) -> i32 { \
           if let 0 = value { 1 } else { 2 } \
         }",
    );
    assert_invariants(&fixture, &ast);

    let ExprKind::IfLet {
        pattern,
        scrutinee: _,
        then_branch,
        else_branch,
    } = tail(&ast, sole_function(&ast)).kind
    else {
        panic!("expected an if let expression");
    };
    assert!(matches!(
        &ast.pattern(pattern).kind,
        PatternKind::Literal(_)
    ));
    assert!(ast.block(then_branch).tail.is_some());
    assert!(
        ast.block(else_branch.expect("the else branch exists"))
            .tail
            .is_some()
    );
}

#[test]
fn else_if_let_chains_keep_each_pattern_in_its_own_branch() {
    let (fixture, ast) = parse_clean(
        "enum Option { None, Some(i32) } \
         func main(value: Option) -> i32 { \
           if let Option::Some(0) = value { 0 } \
           else if let Option::Some(1) = value { 1 } \
           else { 2 } \
         }",
    );
    assert_invariants(&fixture, &ast);

    let ExprKind::IfLet {
        pattern,
        else_branch: Some(outer_else),
        ..
    } = tail(&ast, function(&ast, 1)).kind
    else {
        panic!("expected the first if let");
    };
    let PatternKind::Variant { fields, .. } = &ast.pattern(pattern).kind else {
        panic!("expected a variant pattern");
    };
    assert!(matches!(fields, VariantPatternFields::Positional(_)));

    let nested = ast
        .block(outer_else)
        .tail
        .expect("else if is a block whose tail is the nested expression");
    assert!(matches!(ast.expr(nested).kind, ExprKind::IfLet { .. }));
}

#[test]
fn a_missing_equals_after_the_pattern_is_reported() {
    let fixture = Fixture::new("func main(value: i32) -> i32 { if let 0 value { 0 } else { 1 } }");
    let ast = fixture.parse();
    fixture.expect("E1002", "expected `=`");
    assert_invariants(&fixture, &ast);
}
