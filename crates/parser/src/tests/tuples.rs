//! Tuple types, literals, positional access, and tuple patterns.

use seal_ast::{ExprKind, PatternKind, StmtKind, TypeKind};

use super::support::{Fixture, body, parse_clean, sole_function, tail};

#[test]
fn tuple_types_keep_their_elements_in_order() {
    let (_, ast) = parse_clean("func f(pair: (i32, u8)) -> ((bool, char), ()) { pair }");
    let declaration = sole_function(&ast);
    let parameter = ast.param(declaration.params[0]);
    let TypeKind::Tuple(elements) = &ast.ty(parameter.ty).kind else {
        panic!("expected a tuple parameter type");
    };
    assert_eq!(elements.len(), 2);
    assert!(matches!(
        ast.ty(elements[0]).kind,
        TypeKind::Primitive(seal_ast::PrimitiveType::I32)
    ));
    assert!(matches!(
        ast.ty(elements[1]).kind,
        TypeKind::Primitive(seal_ast::PrimitiveType::U8)
    ));

    let TypeKind::Tuple(elements) = &ast.ty(declaration.ret.expect("return type")).kind else {
        panic!("expected a tuple return type");
    };
    assert_eq!(elements.len(), 2);
    let TypeKind::Tuple(inner) = &ast.ty(elements[0]).kind else {
        panic!("expected a nested tuple");
    };
    assert_eq!(inner.len(), 2);
    let TypeKind::Tuple(empty) = &ast.ty(elements[1]).kind else {
        panic!("expected the unit type");
    };
    assert!(empty.is_empty());
}

#[test]
fn a_parenthesized_type_is_not_a_one_element_tuple() {
    let (_, ast) = parse_clean("func f(value: (i32)) -> i32 { value }");
    let declaration = sole_function(&ast);
    let parameter = ast.param(declaration.params[0]);
    assert!(matches!(
        ast.ty(parameter.ty).kind,
        TypeKind::Primitive(seal_ast::PrimitiveType::I32)
    ));
}

#[test]
fn tuple_literals_keep_source_order_and_trailing_commas() {
    let (_, ast) = parse_clean(
        "func f() { let pair = (1, 2,); let single = (7,); let grouped = (7); let unit = (); }",
    );
    let block = body(&ast, sole_function(&ast));

    let init = |index: usize| match &ast.stmt(block.statements[index]).kind {
        StmtKind::Let(binding) => binding.init.expect("initializer"),
        other => panic!("expected a let, got {other:?}"),
    };

    let ExprKind::Tuple { elements } = &ast.expr(init(0)).kind else {
        panic!("expected a tuple literal");
    };
    assert_eq!(elements.len(), 2);
    assert!(matches!(
        ast.expr(elements[1]).kind,
        ExprKind::Lit(seal_ast::Literal::Int { value: 2, .. })
    ));

    let ExprKind::Tuple { elements } = &ast.expr(init(1)).kind else {
        panic!("expected a one-element tuple");
    };
    assert_eq!(elements.len(), 1);
    assert!(matches!(ast.expr(init(2)).kind, ExprKind::Paren(_)));
    assert!(matches!(
        &ast.expr(init(3)).kind,
        ExprKind::Tuple { elements } if elements.is_empty()
    ));
}

#[test]
fn positional_access_chains_one_field_at_a_time() {
    let (_, ast) = parse_clean("func f(value: ((i32, i32), i32)) -> i32 { value.0.1 }");
    let expression = tail(&ast, sole_function(&ast));
    let ExprKind::TupleField { base, index, .. } = expression.kind else {
        panic!("expected the outer tuple field");
    };
    assert_eq!(index, 1);
    let ExprKind::TupleField { index, .. } = ast.expr(base).kind else {
        panic!("expected the inner tuple field");
    };
    assert_eq!(index, 0);
}

#[test]
fn tuple_indices_are_assignable_targets() {
    let (_, ast) = parse_clean("func f(value: (i32, i32)) { value.1 = 3; }");
    let block = body(&ast, sole_function(&ast));
    let StmtKind::Expr(expression) = ast.stmt(block.statements[0]).kind else {
        panic!("expected an expression statement");
    };
    let ExprKind::Assign { target, .. } = ast.expr(expression).kind else {
        panic!("expected an assignment");
    };
    assert!(matches!(
        ast.expr(target).kind,
        ExprKind::TupleField { index: 1, .. }
    ));
}

#[test]
fn tuple_patterns_nest_and_group_without_trailing_commas() {
    let (_, ast) = parse_clean(
        "func f(value: (i32, (i32, i32))) -> i32 { \
           match value { (first, (second, third)) => first + second + third } \
         }",
    );
    let expression = tail(&ast, sole_function(&ast));
    let ExprKind::Match { arms, .. } = &expression.kind else {
        panic!("expected a match expression");
    };
    let PatternKind::Tuple(elements) = &ast.pattern(arms[0].pattern).kind else {
        panic!("expected a tuple pattern");
    };
    assert_eq!(elements.len(), 2);
    let PatternKind::Tuple(inner) = &ast.pattern(elements[1]).kind else {
        panic!("expected a nested tuple pattern");
    };
    assert_eq!(inner.len(), 2);
}

#[test]
fn a_parenthesized_pattern_is_not_a_one_element_tuple() {
    let (_, ast) = parse_clean("func f(value: i32) -> i32 { match value { (value) => value } }");
    let expression = tail(&ast, sole_function(&ast));
    let ExprKind::Match { arms, .. } = &expression.kind else {
        panic!("expected a match expression");
    };
    assert!(matches!(
        ast.pattern(arms[0].pattern).kind,
        PatternKind::Binding { .. }
    ));
}

#[test]
fn malformed_tuple_syntax_is_reported() {
    let fixture = Fixture::new("func f() { let pair = (1 2); }");
    let _ = fixture.parse();
    fixture.expect("E1002", "expected `)`");

    let fixture = Fixture::new("func f(value: (i32, i32 { }");
    let _ = fixture.parse();
    fixture.expect("E1002", "tuple type");

    let fixture = Fixture::new("func f(value: (i32, i32)) -> i32 { value.0u8 }");
    let _ = fixture.parse();
    fixture.expect("E1002", "without a suffix");
}
