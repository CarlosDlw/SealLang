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
    assert!(area.body.is_none());
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
    assert!(make.body.is_none());
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
fn a_trait_method_may_declare_a_default_body() {
    let (fixture, ast) =
        parse_clean("trait Shape { func area(self: *Self) -> i32 { 0 } func name() -> i32; }");
    let ItemKind::Trait(declaration) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected a trait item");
    };
    assert_eq!(declaration.methods.len(), 2);

    let ItemKind::TraitMethod(area) = &ast.item(declaration.methods[0]).kind else {
        panic!("expected a trait method");
    };
    assert_eq!(fixture.symbol(area.name), "area");
    let body = area.body.expect("area declares a default body");
    let block = ast.block(body);
    assert!(block.statements.is_empty());
    assert!(block.tail.is_some());

    let ItemKind::TraitMethod(name) = &ast.item(declaration.methods[1]).kind else {
        panic!("expected a trait method");
    };
    assert!(name.body.is_none());
}

#[test]
fn a_default_body_keeps_control_flow_and_the_next_method() {
    let (_, ast) = parse_clean(
        "trait Shape { \
             func area(self: *Self) -> i32 { \
                 let base = 1; \
                 if base { 2 } else { 3 } \
             } \
             func name() -> i32; \
         }",
    );
    let ItemKind::Trait(declaration) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected a trait item");
    };
    assert_eq!(declaration.methods.len(), 2);
    let ItemKind::TraitMethod(area) = &ast.item(declaration.methods[0]).kind else {
        panic!("expected a trait method");
    };
    let body = area.body.expect("area declares a default body");
    assert_eq!(ast.block(body).statements.len(), 1);
    assert!(ast.block(body).tail.is_some());
    let ItemKind::TraitMethod(name) = &ast.item(declaration.methods[1]).kind else {
        panic!("expected a trait method");
    };
    assert!(name.body.is_none());
}

#[test]
fn a_default_body_follows_a_where_clause() {
    let (_, ast) =
        parse_clean("trait Shape { func value<T>(self: *Self) -> T where T: Copy { 0 } }");
    let ItemKind::Trait(declaration) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected a trait item");
    };
    let ItemKind::TraitMethod(value) = &ast.item(declaration.methods[0]).kind else {
        panic!("expected a trait method");
    };
    assert!(value.body.is_some());
}

#[test]
fn a_trait_method_signature_ends_with_a_semicolon_or_body() {
    let fixture = Fixture::new("trait Shape { func area(self: *Self) -> i32 }");
    let _ = fixture.parse();
    fixture.expect(
        "E1002",
        "expected `;` or a default body after the trait method signature",
    );
}

#[test]
fn an_orphan_body_after_a_signature_is_recovered() {
    let fixture =
        Fixture::new("trait Shape { func area(self: *Self) -> i32; { 0 } func name() -> i32; }");
    let ast = fixture.parse();
    fixture.expect("E1001", "expected `func` in the trait");
    let ItemKind::Trait(declaration) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected a trait item");
    };
    assert_eq!(declaration.methods.len(), 2);
    let ItemKind::TraitMethod(area) = &ast.item(declaration.methods[0]).kind else {
        panic!("expected a trait method");
    };
    assert!(area.body.is_none());
    let ItemKind::TraitMethod(name) = &ast.item(declaration.methods[1]).kind else {
        panic!("expected a trait method");
    };
    assert!(name.body.is_none());
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
