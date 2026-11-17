//! Enum declarations and qualified variant literals.

use seal_ast::{ExprKind, ItemKind, Literal, PrimitiveType, TypeKind, VariantFieldList};

use crate::tests::support::{Fixture, body, function, parse_clean};

#[test]
fn enum_declarations_keep_variants_and_fields_in_source_order() {
    let (fixture, ast) = parse_clean(
        "enum Shape { Empty, Circle { radius: i32, }, Ring { inner: i32, width: i32, } } \
         func main() {}",
    );
    let ItemKind::Enum(declaration) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected an enum item");
    };
    assert_eq!(fixture.symbol(declaration.name), "Shape");
    assert_eq!(declaration.variants.len(), 3);

    let empty = ast.variant(declaration.variants[0]);
    assert_eq!(fixture.symbol(empty.name), "Empty");
    assert!(matches!(&empty.fields, VariantFieldList::Unit));

    let circle = ast.variant(declaration.variants[1]);
    assert_eq!(fixture.symbol(circle.name), "Circle");
    let VariantFieldList::Named(circle_fields) = &circle.fields else {
        panic!("expected named fields");
    };
    assert_eq!(circle_fields.len(), 1);
    let radius = ast.field(circle_fields[0]);
    assert_eq!(fixture.symbol(radius.name), "radius");
    assert!(matches!(
        ast.ty(radius.ty).kind,
        TypeKind::Primitive(PrimitiveType::I32)
    ));

    let ring = ast.variant(declaration.variants[2]);
    let VariantFieldList::Named(ring_fields) = &ring.fields else {
        panic!("expected named fields");
    };
    assert_eq!(ring_fields.len(), 2);
    assert_eq!(fixture.symbol(ast.field(ring_fields[0]).name), "inner");
    assert_eq!(fixture.symbol(ast.field(ring_fields[1]).name), "width");
}

#[test]
fn qualified_variant_literals_keep_their_initializers() {
    let (fixture, ast) = parse_clean(
        "enum Shape { Empty, Circle { radius: i32 } } \
         func main() -> Shape { Shape::Circle { radius: 1 } }",
    );
    let declaration = function(&ast, 1);
    let tail = body(&ast, declaration).tail.expect("tail exists");
    let ExprKind::Variant {
        type_name,
        type_span,
        name,
        name_span,
        fields,
    } = &ast.expr(tail).kind
    else {
        panic!("expected a variant literal");
    };
    assert_eq!(fixture.symbol(*type_name), "Shape");
    assert_eq!(fixture.text(*type_span), "Shape");
    assert_eq!(fixture.symbol(*name), "Circle");
    assert_eq!(fixture.text(*name_span), "Circle");
    assert_eq!(fields.len(), 1);
    assert_eq!(fixture.symbol(fields[0].name), "radius");
    assert!(matches!(
        ast.expr(fields[0].value).kind,
        ExprKind::Lit(Literal::Int { value: 1, .. })
    ));
}

#[test]
fn a_unit_variant_path_is_still_a_qualified_path() {
    let (fixture, ast) = parse_clean("enum Shape { Empty } func main() -> Shape { Shape::Empty }");
    let declaration = function(&ast, 1);
    let tail = body(&ast, declaration).tail.expect("tail exists");
    let ExprKind::PathQualified {
        type_name, name, ..
    } = &ast.expr(tail).kind
    else {
        panic!("expected a qualified unit variant path");
    };
    assert_eq!(fixture.symbol(*type_name), "Shape");
    assert_eq!(fixture.symbol(*name), "Empty");
}

#[test]
fn tuple_variant_fields_are_declared_by_type_only() {
    let (fixture, ast) = parse_clean("enum Option { Some(i32, *i32), None } func main() {}");
    let ItemKind::Enum(declaration) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected an enum item");
    };
    assert_eq!(
        fixture.symbol(ast.variant(declaration.variants[0]).name),
        "Some"
    );
    assert_eq!(declaration.variants.len(), 2);
    let VariantFieldList::Tuple(fields) = &ast.variant(declaration.variants[0]).fields else {
        panic!("expected positional fields");
    };
    assert_eq!(fields.len(), 2);
    assert!(matches!(
        ast.ty(ast.tuple_field(fields[0]).ty).kind,
        TypeKind::Primitive(PrimitiveType::I32)
    ));
    assert!(matches!(
        ast.ty(ast.tuple_field(fields[1]).ty).kind,
        TypeKind::Pointer { .. }
    ));
    assert!(matches!(
        &ast.variant(declaration.variants[1]).fields,
        VariantFieldList::Unit
    ));
}

#[test]
fn a_missing_variant_comma_is_diagnosed() {
    let fixture = Fixture::new("enum Mode { On Off } func main() {}");
    let ast = fixture.parse();
    fixture.expect("E1002", "between variants");
    let ItemKind::Enum(declaration) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected an enum item");
    };
    assert_eq!(declaration.variants.len(), 2);
}

#[test]
fn an_unclosed_enum_is_diagnosed() {
    let fixture = Fixture::new("enum Mode { On { value: i32 }");
    let _ = fixture.parse();
    fixture.expect("E1002", "close the enum");
}
