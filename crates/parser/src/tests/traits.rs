//! Trait declarations and `impl Trait for Type` blocks.

use seal_ast::{ItemKind, TypeKind};

use super::support::{Fixture, parse_clean};

#[test]
fn traits_group_signatures_and_reify_self() {
    let (fixture, ast) = parse_clean(
        "struct Circle { radius: i32 } \
         trait Shape { \
             func area(self: *Self) -> i32; \
             func make() -> Self; \
         }",
    );

    assert_eq!(ast.program().len(), 2);
    let ItemKind::Trait(declaration) = &ast.item(ast.program().items()[1]).kind else {
        panic!("expected a trait item");
    };
    assert_eq!(fixture.symbol(declaration.name), "Shape");
    assert_eq!(declaration.methods.len(), 2);

    let ItemKind::TraitMethod(area) = &ast.item(declaration.methods[0]).kind else {
        panic!("expected a trait method");
    };
    assert_eq!(fixture.symbol(area.name), "area");
    assert_eq!(area.params.len(), 1);
    let receiver = ast.param(area.params[0]);
    assert_eq!(fixture.symbol(receiver.name), "self");
    let TypeKind::Pointer { pointee, mutable } = ast.ty(receiver.ty).kind else {
        panic!("expected a pointer receiver");
    };
    assert!(!mutable);
    assert!(matches!(ast.ty(pointee).kind, TypeKind::Path { .. }));

    let ItemKind::TraitMethod(make) = &ast.item(declaration.methods[1]).kind else {
        panic!("expected a trait method");
    };
    assert!(make.params.is_empty());
    assert!(make.ret.is_some());
}

#[test]
fn impl_blocks_record_the_trait_they_implement() {
    let (fixture, ast) = parse_clean(
        "struct Circle { radius: i32 } \
         trait Shape { func area(self: *Self) -> i32; } \
         impl Shape for Circle { func area(self: *Circle) -> i32 { (*self).radius } }",
    );

    assert_eq!(ast.program().len(), 3);
    let ItemKind::Impl(declaration) = &ast.item(ast.program().items()[2]).kind else {
        panic!("expected an impl item");
    };
    let trait_ref = declaration
        .trait_ref
        .as_ref()
        .expect("the impl names a trait");
    assert_eq!(fixture.symbol(trait_ref.name), "Shape");
    let TypeKind::Path(self_path) = &ast.ty(declaration.self_ty).kind else {
        panic!("expected a named impl target");
    };
    assert_eq!(fixture.symbol(self_path.name), "Circle");
    assert_eq!(declaration.functions.len(), 1);
}

#[test]
fn inherent_impls_have_no_trait() {
    let (_, ast) = parse_clean(
        "struct Point { x: i32 } impl Point { func get(self: *Point) -> i32 { (*self).x } }",
    );
    let ItemKind::Impl(declaration) = &ast.item(ast.program().items()[1]).kind else {
        panic!("expected an impl item");
    };
    assert!(declaration.trait_ref.is_none());
}

#[test]
fn a_trait_method_cannot_declare_a_body() {
    let fixture = Fixture::new("trait Shape { func area(self: *Self) -> i32 { 0 } }");
    let ast = fixture.parse();
    fixture.expect("E1002", "declares only its signature");
    let ItemKind::Trait(declaration) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected a trait item");
    };
    assert_eq!(declaration.methods.len(), 1);
}

#[test]
fn a_trait_method_signature_ends_with_a_semicolon() {
    let fixture = Fixture::new("trait Shape { func area(self: *Self) -> i32 }");
    let _ = fixture.parse();
    fixture.expect("E1002", "expected `;` after the trait method signature");
}

#[test]
fn malformed_trait_members_are_diagnosed() {
    let fixture = Fixture::new("trait Shape { let x = 1; }");
    let ast = fixture.parse();
    fixture.expect("E1001", "expected `func` in the trait");
    let ItemKind::Trait(declaration) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected a trait item");
    };
    assert!(declaration.methods.is_empty());
}
