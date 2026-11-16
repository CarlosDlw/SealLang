//! Struct declarations, literals, and field access.

use seal_ast::{ExprKind, ItemKind, Literal, PrimitiveType, StmtKind, TypeKind};

use crate::tests::support::{Fixture, body, parse_clean, sole_function};

#[test]
fn struct_declarations_keep_fields_in_source_order() {
    let (fixture, ast) = parse_clean("struct Point { x: i32, y: *i32, } func main() {}");
    assert_eq!(ast.program().len(), 2);
    let ItemKind::Struct(declaration) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected a struct item");
    };
    assert_eq!(fixture.symbol(declaration.name), "Point");
    assert_eq!(declaration.fields.len(), 2);

    let x = ast.field(declaration.fields[0]);
    assert_eq!(fixture.symbol(x.name), "x");
    assert!(matches!(
        ast.ty(x.ty).kind,
        TypeKind::Primitive(PrimitiveType::I32)
    ));

    let y = ast.field(declaration.fields[1]);
    assert_eq!(fixture.symbol(y.name), "y");
    match &ast.ty(y.ty).kind {
        TypeKind::Pointer { pointee, mutable } => {
            assert!(!mutable);
            assert!(matches!(
                ast.ty(*pointee).kind,
                TypeKind::Primitive(PrimitiveType::I32)
            ));
        }
        other => panic!("expected a pointer field, got {other:?}"),
    }
}

#[test]
fn named_types_are_recorded_for_resolution() {
    let (fixture, ast) = parse_clean("struct Node { next: *Node } func main() {}");
    let ItemKind::Struct(declaration) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected a struct item");
    };
    let next = ast.field(declaration.fields[0]);
    match &ast.ty(next.ty).kind {
        TypeKind::Pointer { pointee, .. } => match ast.ty(*pointee).kind {
            TypeKind::Path(symbol) => assert_eq!(fixture.symbol(symbol), "Node"),
            ref other => panic!("expected a named pointee, got {other:?}"),
        },
        other => panic!("expected a pointer, got {other:?}"),
    }
}

#[test]
fn empty_structs_parse() {
    let (_, ast) = parse_clean("struct Empty {} func main() {}");
    let ItemKind::Struct(declaration) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected a struct item");
    };
    assert!(declaration.fields.is_empty());
}

#[test]
fn field_access_chains_bind_tighter_than_binary_operators() {
    let (fixture, ast) = parse_clean("func main() { p.x.y + q }");
    let declaration = sole_function(&ast);
    let tail = body(&ast, declaration).tail.expect("tail exists");
    let ExprKind::Binary { lhs, .. } = &ast.expr(tail).kind else {
        panic!("expected a binary tail");
    };
    let ExprKind::Field { base, name, .. } = &ast.expr(*lhs).kind else {
        panic!("expected an outer field access");
    };
    assert_eq!(fixture.symbol(*name), "y");
    let ExprKind::Field { base, name, .. } = &ast.expr(*base).kind else {
        panic!("expected an inner field access");
    };
    assert_eq!(fixture.symbol(*name), "x");
    assert!(matches!(ast.expr(*base).kind, ExprKind::Path(_)));
}

#[test]
fn struct_literals_keep_source_order() {
    let (fixture, ast) = parse_clean("func main() { Point { y: 1, x: 2 } }");
    let declaration = sole_function(&ast);
    let tail = body(&ast, declaration).tail.expect("tail exists");
    let ExprKind::Struct {
        name,
        name_span,
        fields,
    } = &ast.expr(tail).kind
    else {
        panic!("expected a struct literal");
    };
    assert_eq!(fixture.symbol(*name), "Point");
    assert_eq!(fixture.text(*name_span), "Point");
    assert_eq!(fields.len(), 2);
    assert_eq!(fixture.symbol(fields[0].name), "y");
    assert_eq!(fixture.symbol(fields[1].name), "x");
    assert!(matches!(
        ast.expr(fields[0].value).kind,
        ExprKind::Lit(Literal::Int { value: 1, .. })
    ));
}

#[test]
fn struct_literals_in_conditions_require_parentheses() {
    let (fixture, ast) =
        parse_clean("func main() -> i32 { if (Point { x: 1 }).x == 1 { 1 } else { 0 } }");
    let declaration = sole_function(&ast);
    let tail = body(&ast, declaration).tail.expect("tail exists");
    let ExprKind::If { condition, .. } = &ast.expr(tail).kind else {
        panic!("expected an if");
    };
    let ExprKind::Binary { lhs, .. } = &ast.expr(*condition).kind else {
        panic!("expected a binary condition");
    };
    let ExprKind::Field { base, name, .. } = &ast.expr(*lhs).kind else {
        panic!("expected a field access");
    };
    assert_eq!(fixture.symbol(*name), "x");
    assert!(matches!(ast.expr(*base).kind, ExprKind::Paren(_)));

    let fixture = Fixture::new("func main() { if Point { x: 1 } { } }");
    let _ = fixture.parse();
    fixture.expect("E1002", "`;`");
}

#[test]
fn field_assignment_targets_are_syntactically_assignable() {
    let (_, ast) = parse_clean("func main() { p.x = 1; }");
    let declaration = sole_function(&ast);
    let block = body(&ast, declaration);
    let StmtKind::Expr(statement) = &ast.stmt(block.statements[0]).kind else {
        panic!("expected an expression statement");
    };
    assert!(matches!(ast.expr(*statement).kind, ExprKind::Assign { .. }));
}

#[test]
fn a_missing_field_comma_is_diagnosed() {
    let fixture = Fixture::new("struct Point { x: i32 y: i32 } func main() {}");
    let ast = fixture.parse();
    fixture.expect("E1002", "between fields");
    let ItemKind::Struct(declaration) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected a struct item");
    };
    assert_eq!(declaration.fields.len(), 2);
}

#[test]
fn a_missing_field_colon_is_diagnosed() {
    let fixture = Fixture::new("struct Point { x i32 } func main() {}");
    let _ = fixture.parse();
    fixture.expect("E1002", "`:`");
}

#[test]
fn an_unclosed_struct_is_diagnosed() {
    let fixture = Fixture::new("struct Point { x: i32");
    let _ = fixture.parse();
    fixture.expect("E1002", "`}`");
}
