//! Array types, literals, repeats, and indexing.

use seal_ast::{ExprKind, StmtKind, TypeKind};

use super::support::{Fixture, body, parse_clean, sole_function, tail};

#[test]
fn array_types_carry_the_element_and_length() {
    let (fixture, ast) = parse_clean("func f(a: [i32; 4]) -> [[u8; 2]; 3] { a }");
    let declaration = sole_function(&ast);
    let param = ast.param(declaration.params[0]);
    let TypeKind::Array {
        element,
        length,
        length_span,
    } = ast.ty(param.ty).kind
    else {
        panic!("expected an array parameter type");
    };
    assert_eq!(length, 4);
    assert_eq!(fixture.text(length_span), "4");
    assert!(matches!(
        ast.ty(element).kind,
        TypeKind::Primitive(seal_ast::PrimitiveType::I32)
    ));

    let TypeKind::Array {
        element, length, ..
    } = ast.ty(declaration.ret.expect("return type")).kind
    else {
        panic!("expected an array return type");
    };
    assert_eq!(length, 3);
    let TypeKind::Array {
        element, length, ..
    } = ast.ty(element).kind
    else {
        panic!("expected a nested array element");
    };
    assert_eq!(length, 2);
    assert!(matches!(
        ast.ty(element).kind,
        TypeKind::Primitive(seal_ast::PrimitiveType::U8)
    ));
}

#[test]
fn array_literals_keep_source_order_and_allow_a_trailing_comma() {
    let (fixture, ast) = parse_clean("func f() { let a = [1, 2, 3,]; let b = []; }");
    let block = body(&ast, sole_function(&ast));
    let first = match &ast.stmt(block.statements[0]).kind {
        StmtKind::Let(binding) => binding.init.expect("initializer"),
        other => panic!("expected a let, got {other:?}"),
    };
    let ExprKind::Array { elements } = &ast.expr(first).kind else {
        panic!("expected an array literal");
    };
    assert_eq!(elements.len(), 3);

    let empty = match &ast.stmt(block.statements[1]).kind {
        StmtKind::Let(binding) => binding.init.expect("initializer"),
        other => panic!("expected a let, got {other:?}"),
    };
    assert!(matches!(
        ast.expr(empty).kind,
        ExprKind::Array { ref elements } if elements.is_empty()
    ));
    let _ = fixture;
}

#[test]
fn a_repeated_literal_keeps_its_value_and_count() {
    let (fixture, ast) = parse_clean("func f() { let a = [7; 5]; }");
    let block = body(&ast, sole_function(&ast));
    let init = match &ast.stmt(block.statements[0]).kind {
        StmtKind::Let(binding) => binding.init.expect("initializer"),
        other => panic!("expected a let, got {other:?}"),
    };
    let ExprKind::ArrayRepeat {
        value,
        length,
        length_span,
    } = ast.expr(init).kind
    else {
        panic!("expected a repeated array literal");
    };
    assert_eq!(length, 5);
    assert_eq!(fixture.text(length_span), "5");
    assert!(matches!(
        ast.expr(value).kind,
        ExprKind::Lit(seal_ast::Literal::Int { value: 7, .. })
    ));
}

#[test]
fn indexing_is_postfix_and_chains() {
    let (fixture, ast) = parse_clean("func f(a: [[i32; 2]; 2]) -> i32 { a[0][1] }");
    let expression = tail(&ast, sole_function(&ast));
    let ExprKind::Index { base, index } = expression.kind else {
        panic!("expected an outer index");
    };
    assert!(matches!(
        ast.expr(index).kind,
        ExprKind::Lit(seal_ast::Literal::Int { value: 1, .. })
    ));
    let ExprKind::Index { index, .. } = ast.expr(base).kind else {
        panic!("expected an inner index");
    };
    assert!(matches!(
        ast.expr(index).kind,
        ExprKind::Lit(seal_ast::Literal::Int { value: 0, .. })
    ));
    let _ = fixture;
}

#[test]
fn array_indices_are_assignable_targets() {
    let (_, ast) = parse_clean("func f(a: [i32; 2]) { a[0] = 1; }");
    let block = body(&ast, sole_function(&ast));
    let StmtKind::Expr(expression) = ast.stmt(block.statements[0]).kind else {
        panic!("expected an expression statement");
    };
    assert!(matches!(ast.expr(expression).kind, ExprKind::Assign { .. }));
}

#[test]
fn malformed_array_lengths_report_e1009() {
    let fixture = Fixture::new("func f(a: [i32; n]) {}");
    let _ = fixture.parse();
    fixture.expect("E1009", "expected an array length");

    let fixture = Fixture::new("func f() { let a = [1; 2u8]; }");
    let _ = fixture.parse();
    fixture.expect("E1009", "must be `usize`");

    let fixture = Fixture::new("func f(a: [i32; 4294967296]) {}");
    let _ = fixture.parse();
    fixture.expect("E1009", "too large");
}

#[test]
fn a_missing_array_closer_is_reported() {
    let fixture = Fixture::new("func f(a: [i32; 2) {}");
    let _ = fixture.parse();
    fixture.expect("E1002", "`]`");

    let fixture = Fixture::new("func f() { let a = [1, 2; }");
    let _ = fixture.parse();
    fixture.expect("E1002", "`]`");
}
