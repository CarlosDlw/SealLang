//! Slice types and range syntax.

use seal_ast::{ExprKind, StmtKind, TypeKind};

use super::support::{Fixture, body, parse_clean, sole_function};

fn range_bounds(source: &str) -> (bool, bool) {
    let (_, ast) = parse_clean(source);
    let declaration = sole_function(&ast);
    let block = body(&ast, declaration);
    let init = match &ast.stmt(block.statements[0]).kind {
        StmtKind::Let(binding) => binding.init.expect("initializer"),
        other => panic!("expected a let, got {other:?}"),
    };
    let ExprKind::Borrow { operand, .. } = ast.expr(init).kind else {
        panic!("expected a borrow, got {:?}", ast.expr(init).kind);
    };
    let ExprKind::Slice { start, end, .. } = ast.expr(operand).kind else {
        panic!("expected a slice range, got {:?}", ast.expr(operand).kind);
    };
    (start.is_some(), end.is_some())
}

#[test]
fn slice_types_keep_their_element() {
    let (_, ast) = parse_clean("func f(a: [i32]) -> [u8] { a }");
    let declaration = sole_function(&ast);
    let param = ast.param(declaration.params[0]);
    let TypeKind::Slice { element } = ast.ty(param.ty).kind else {
        panic!("expected a slice parameter");
    };
    assert!(matches!(
        ast.ty(element).kind,
        TypeKind::Primitive(seal_ast::PrimitiveType::I32)
    ));

    let TypeKind::Slice { element } = ast.ty(declaration.ret.expect("return type")).kind else {
        panic!("expected a slice return type");
    };
    assert!(matches!(
        ast.ty(element).kind,
        TypeKind::Primitive(seal_ast::PrimitiveType::U8)
    ));
}

#[test]
fn every_range_form_keeps_its_bounds() {
    let cases = [
        ("func f(a: [i32; 4]) { let x = &a[..]; }", false, false),
        ("func f(a: [i32; 4]) { let x = &a[..2]; }", false, true),
        ("func f(a: [i32; 4]) { let x = &a[1..]; }", true, false),
        ("func f(a: [i32; 4]) { let x = &a[1..2]; }", true, true),
    ];
    for (source, start, end) in cases {
        assert_eq!(range_bounds(source), (start, end), "{source}");
    }
}

#[test]
fn a_malformed_range_reports_a_missing_bracket() {
    let fixture = Fixture::new("func f(a: [i32; 4]) { let x = &a[1..2; }");
    let _ = fixture.parse();
    fixture.expect("E1002", "`]`");

    let fixture = Fixture::new("func f(a: [i32) {}");
    let _ = fixture.parse();
    fixture.expect("E1002", "`]`");
}
