//! Visibility on items, fields, and inherent methods.

use seal_ast::{ItemKind, Visibility};

use crate::tests::support::{Fixture, assert_invariants, parse_clean};

#[test]
fn item_visibility_is_recorded_for_every_level() {
    let (fixture, ast) = parse_clean(
        "public func public_fn() {}\n\
         internal struct Token {}\n\
         private struct Secret {}\n\
         public trait Shape { func area() -> i32; }",
    );
    let items = ast.program().items();
    assert_eq!(ast.item(items[0]).modifiers.visibility, Visibility::Public);
    assert_eq!(
        ast.item(items[1]).modifiers.visibility,
        Visibility::Internal
    );
    assert_eq!(ast.item(items[2]).modifiers.visibility, Visibility::Private);
    assert!(ast.item(items[2]).modifiers.has_visibility);
    assert_eq!(ast.item(items[3]).modifiers.visibility, Visibility::Public);
    assert_invariants(&fixture, &ast);
}

#[test]
fn struct_fields_carry_their_own_visibility() {
    let (fixture, ast) =
        parse_clean("public struct Point { public x: i32, internal y: i32, z: i32 }");
    let ItemKind::Struct(declaration) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected a struct");
    };
    let visibilities: Vec<Visibility> = declaration
        .fields
        .iter()
        .map(|field| ast.field(*field).visibility)
        .collect();
    assert_eq!(
        visibilities,
        [
            Visibility::Public,
            Visibility::Internal,
            Visibility::Private
        ]
    );
    assert_invariants(&fixture, &ast);
}

#[test]
fn inherent_methods_accept_visibility_but_trait_methods_do_not() {
    let (fixture, ast) = parse_clean(
        "struct Point {}\n\
         impl Point { public func x() {} internal func y() {} private func z() {} }\n\
         trait Shape { func area() -> i32; }",
    );
    let items = ast.program().items();
    let ItemKind::Impl(implementation) = &ast.item(items[1]).kind else {
        panic!("expected an impl");
    };
    assert_eq!(
        ast.item(implementation.functions[0]).modifiers.visibility,
        Visibility::Public
    );
    assert_eq!(
        ast.item(implementation.functions[1]).modifiers.visibility,
        Visibility::Internal
    );
    assert_eq!(
        ast.item(implementation.functions[2]).modifiers.visibility,
        Visibility::Private
    );
    assert_invariants(&fixture, &ast);

    let rejected = Fixture::new("trait Shape { public func area() -> i32; }");
    rejected.parse();
    assert_eq!(rejected.codes(), ["E2079"]);
}

#[test]
fn visibility_on_a_variant_is_rejected_with_one_diagnostic() {
    let fixture = Fixture::new("enum Shape { public Circle }");
    let ast = fixture.parse();
    assert_eq!(fixture.codes(), ["E2079"]);
    let ItemKind::Enum(declaration) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected an enum");
    };
    assert_eq!(declaration.variants.len(), 1);
}

#[test]
fn visibility_on_an_impl_is_rejected_with_one_diagnostic() {
    for source in ["public impl Point {}", "private impl Point {}"] {
        let fixture = Fixture::new(source);
        let ast = fixture.parse();
        assert_eq!(fixture.codes(), ["E2079"], "source: {source}");
        assert!(matches!(
            ast.item(ast.program().items()[0]).kind,
            ItemKind::Impl(_)
        ));
    }
}

#[test]
fn cfg_on_a_field_is_rejected_and_the_field_survives() {
    let fixture = Fixture::new("struct S { cfg(feature(\"x\")) value: i32 }");
    let ast = fixture.parse();
    assert_eq!(fixture.codes(), ["E2085"]);
    let ItemKind::Struct(declaration) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected a struct");
    };
    assert_eq!(declaration.fields.len(), 1);
    assert_invariants(&fixture, &ast);
}

#[test]
fn cfg_on_a_variant_is_rejected_and_the_variant_survives() {
    let fixture = Fixture::new("enum E { cfg(feature(\"x\")) V }");
    let ast = fixture.parse();
    assert_eq!(fixture.codes(), ["E2085"]);
    let ItemKind::Enum(declaration) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected an enum");
    };
    assert_eq!(declaration.variants.len(), 1);
    assert_invariants(&fixture, &ast);
}

#[test]
fn test_applies_only_to_functions() {
    let (fixture, ast) = parse_clean("test func focused() {}");
    assert!(ast.item(ast.program().items()[0]).modifiers.is_test);
    assert_invariants(&fixture, &ast);

    let rejected = Fixture::new("test struct S {}");
    rejected.parse();
    assert_eq!(rejected.codes(), ["E1001"]);
}

#[test]
fn duplicate_visibility_is_one_error() {
    let fixture = Fixture::new("public internal func f() {}");
    let ast = fixture.parse();
    assert_eq!(fixture.codes(), ["E1001"]);
    assert_eq!(
        ast.item(ast.program().items()[0]).modifiers.visibility,
        Visibility::Public
    );
}
