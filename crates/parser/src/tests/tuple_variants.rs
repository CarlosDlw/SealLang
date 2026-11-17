//! Positional enum variant fields and their recovery paths.

use seal_ast::{ExprKind, ItemKind, PrimitiveType, TypeKind, VariantFieldList};

use crate::tests::support::{Fixture, assert_invariants, body, function, parse_clean};

#[test]
fn positional_fields_keep_their_types_in_order() {
    let (fixture, ast) = parse_clean("enum Option { Some(i32, *i32), None } func main() {}");
    let ItemKind::Enum(declaration) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected an enum item");
    };
    let VariantFieldList::Tuple(fields) = &ast.variant(declaration.variants[0]).fields else {
        panic!("expected positional fields");
    };
    assert_eq!(
        fixture.symbol(ast.variant(declaration.variants[0]).name),
        "Some"
    );
    assert_eq!(fields.len(), 2);
    assert!(matches!(
        ast.ty(ast.tuple_field(fields[0]).ty).kind,
        TypeKind::Primitive(PrimitiveType::I32)
    ));
    assert!(matches!(
        ast.ty(ast.tuple_field(fields[1]).ty).kind,
        TypeKind::Pointer { .. }
    ));
}

#[test]
fn a_positional_variant_is_constructed_with_parentheses() {
    let (fixture, ast) =
        parse_clean("enum Option { Some(i32) } func main() -> Option { Option::Some(1) }");
    let declaration = function(&ast, 1);
    let tail = body(&ast, declaration).tail.expect("tail exists");
    let ExprKind::Call { callee, args } = &ast.expr(tail).kind else {
        panic!("expected a call");
    };
    assert_eq!(args.len(), 1);
    let ExprKind::PathQualified {
        type_name, name, ..
    } = &ast.expr(*callee).kind
    else {
        panic!("expected a qualified path");
    };
    assert_eq!(fixture.symbol(*type_name), "Option");
    assert_eq!(fixture.symbol(*name), "Some");
}

#[test]
fn an_empty_positional_list_is_diagnosed_as_a_unit_variant() {
    let fixture = Fixture::new("enum Option { Some(), None } func main() {}");
    let ast = fixture.parse();
    fixture.expect("E1010", "empty positional");
    assert_invariants(&fixture, &ast);
    let ItemKind::Enum(declaration) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected an enum item");
    };
    assert!(matches!(
        &ast.variant(declaration.variants[0]).fields,
        VariantFieldList::Unit
    ));
}

#[test]
fn a_named_field_inside_parentheses_is_diagnosed() {
    let fixture = Fixture::new("enum Option { Some(value: i32), None } func main() {}");
    let ast = fixture.parse();
    fixture.expect("E1010", "has no name");
    assert_invariants(&fixture, &ast);
    let ItemKind::Enum(declaration) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected an enum item");
    };
    let VariantFieldList::Tuple(fields) = &ast.variant(declaration.variants[0]).fields else {
        panic!("expected positional fields");
    };
    assert_eq!(fields.len(), 1);
    assert!(matches!(
        ast.ty(ast.tuple_field(fields[0]).ty).kind,
        TypeKind::Primitive(PrimitiveType::I32)
    ));
}

#[test]
fn a_mixed_field_list_is_diagnosed_without_leaving_orphan_nodes() {
    let fixture =
        Fixture::new("enum Option { Some(i32, i32) { value: i32 }, None } func main() {}");
    let ast = fixture.parse();
    fixture.expect("E1010", "both positional and named");
    assert_invariants(&fixture, &ast);
    let ItemKind::Enum(declaration) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected an enum item");
    };
    let VariantFieldList::Tuple(fields) = &ast.variant(declaration.variants[0]).fields else {
        panic!("expected positional fields");
    };
    assert_eq!(fields.len(), 2);
}

#[test]
fn a_missing_positional_comma_is_diagnosed_and_recovered() {
    let fixture = Fixture::new("enum Pair { Both(i32 i32), Empty } func main() {}");
    let ast = fixture.parse();
    fixture.expect("E1002", "positional field list");
    assert_invariants(&fixture, &ast);
    let ItemKind::Enum(declaration) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected an enum item");
    };
    let VariantFieldList::Tuple(fields) = &ast.variant(declaration.variants[0]).fields else {
        panic!("expected positional fields");
    };
    assert_eq!(fields.len(), 2);
}
