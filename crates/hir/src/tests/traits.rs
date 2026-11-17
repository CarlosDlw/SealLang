//! Trait registration, contract validation, and static lookup.

use seal_def::DefKind;

use crate::{ExprKind, ReceiverKind, Res, TraitMethodLookup};

use super::support::{Fixture, function, lower_clean, path_res, tail};

const SHAPE: &str = "\
struct Circle { radius: i32 } \
trait Shape { \
    func area(self: *Self) -> i32; \
    func scale(self: *Self, factor: i32) -> i32; \
    func unit() -> i32; \
} \
impl Shape for Circle { \
    func area(self: *Circle) -> i32 { (*self).radius * (*self).radius } \
    func scale(self: *Circle, factor: i32) -> i32 { (*self).radius * factor } \
    func unit() -> i32 { 1 } \
}";

#[test]
fn trait_impls_bind_each_method_to_a_concrete_function() {
    let (fixture, hir) = lower_clean(SHAPE);
    let circle = type_named(&fixture, &hir, "Circle");
    let area = fixture.interner.intern("area");
    let TraitMethodLookup::Found {
        trait_def,
        function,
    } = hir.trait_method(circle, area)
    else {
        panic!("area is implemented");
    };
    assert_eq!(hir.definitions().kind(trait_def), DefKind::Trait);
    assert_eq!(hir.definitions().get(function).parent, Some(circle));
    assert_eq!(hir.receiver(function), Some(ReceiverKind::Shared));
    assert_eq!(hir.bodies().len(), 3);
}

#[test]
fn trait_methods_are_not_in_the_inherent_registry() {
    let (fixture, hir) = lower_clean(SHAPE);
    let circle = type_named(&fixture, &hir, "Circle");
    assert!(
        hir.method(circle, fixture.interner.intern("area"))
            .is_none()
    );
}

#[test]
fn a_trait_associated_function_resolves_through_the_type() {
    let (fixture, hir) = lower_clean(&format!("{SHAPE} func main() -> i32 {{ Circle::unit() }}"));
    let circle = type_named(&fixture, &hir, "Circle");
    let TraitMethodLookup::Found { function: unit, .. } =
        hir.trait_method(circle, fixture.interner.intern("unit"))
    else {
        panic!("unit is implemented");
    };
    let (_, body) = function(&fixture, &hir, "main");
    let ExprKind::Call { callee, .. } = hir.expr(tail(&hir, body)).kind.clone() else {
        panic!("expected a call");
    };
    assert_eq!(path_res(&hir, callee), Res::Function(unit));
}

#[test]
fn a_trait_method_cannot_be_called_through_the_type() {
    let fixture = Fixture::new(&format!("{SHAPE} func main() -> i32 {{ Circle::area() }}"));
    let _ = fixture.lower();
    fixture.expect("E2019", "`area` is a method");
}

#[test]
fn a_trait_is_not_a_type() {
    let fixture = Fixture::new("trait Shape {} struct Circle { radius: Shape }");
    let _ = fixture.lower();
    fixture.expect("E2027", "is a trait, not a type");
}

#[test]
fn a_trait_is_not_a_value() {
    let fixture = Fixture::new("trait Shape {} func main() { let shape = Shape; }");
    let _ = fixture.lower();
    fixture.expect("E2027", "is a trait, not a value");
}

#[test]
fn self_is_rejected_outside_a_trait_declaration() {
    let fixture = Fixture::new("func clone(value: Self) {}");
    let _ = fixture.lower();
    fixture.expect("E2059", "only allowed in a trait or impl context");
}

#[test]
fn a_missing_trait_method_is_reported() {
    let fixture = Fixture::new(
        "struct Circle { radius: i32 } \
         trait Shape { func area(self: *Self) -> i32; } \
         impl Shape for Circle {}",
    );
    let _ = fixture.lower();
    fixture.expect("E2022", "requires `area`");
}

#[test]
fn an_extra_function_in_a_trait_impl_is_reported() {
    let fixture = Fixture::new(
        "struct Circle { radius: i32 } \
         trait Shape { func area(self: *Self) -> i32; } \
         impl Shape for Circle { \
             func area(self: *Circle) -> i32 { (*self).radius } \
             func extra() -> i32 { 1 } \
         }",
    );
    let _ = fixture.lower();
    fixture.expect("E2023", "does not declare `extra`");
}

#[test]
fn a_parameter_type_must_match_the_trait() {
    let fixture = Fixture::new(
        "struct Circle { radius: i32 } \
         trait Shape { func grow(self: *Self, factor: i32) -> i32; } \
         impl Shape for Circle { func grow(self: *Circle, factor: i64) -> i32 { 0 } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2024", "parameter 1 has type `i64`");
}

#[test]
fn a_return_type_must_match_the_trait() {
    let fixture = Fixture::new(
        "struct Circle { radius: i32 } \
         trait Shape { func area(self: *Self) -> i32; } \
         impl Shape for Circle { func area(self: *Circle) -> i64 { 0 } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2024", "return type is `i64`");
}

#[test]
fn a_receiver_must_match_the_trait() {
    let fixture = Fixture::new(
        "struct Circle { radius: i32 } \
         trait Shape { func area(self: *Self) -> i32; } \
         impl Shape for Circle { func area(self: *mut Circle) -> i32 { 0 } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2024", "receiver does not match");
}

#[test]
fn a_parameter_count_must_match_the_trait() {
    let fixture = Fixture::new(
        "struct Circle { radius: i32 } \
         trait Shape { func area(self: *Self) -> i32; } \
         impl Shape for Circle { func area(self: *Circle, factor: i32) -> i32 { 0 } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2024", "the trait declares 0 parameter(s)");
}

#[test]
fn a_type_implements_a_trait_only_once() {
    let fixture = Fixture::new(
        "struct Circle { radius: i32 } \
         trait Shape { func area(self: *Self) -> i32; } \
         impl Shape for Circle { func area(self: *Circle) -> i32 { 1 } } \
         impl Shape for Circle { func area(self: *Circle) -> i32 { 2 } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2021", "already implemented");
}

#[test]
fn an_unknown_trait_is_reported() {
    let fixture = Fixture::new("struct Circle { radius: i32 } impl Missing for Circle {}");
    let _ = fixture.lower();
    fixture.expect("E2020", "cannot find trait `Missing`");
}

#[test]
fn a_struct_cannot_be_used_as_a_trait() {
    let fixture = Fixture::new("struct Circle { radius: i32 } impl Circle for Circle {}");
    let _ = fixture.lower();
    fixture.expect("E2020", "is a type, not a trait");
}

#[test]
fn a_trait_method_is_declared_once() {
    let fixture = Fixture::new(
        "trait Shape { func area(self: *Self) -> i32; func area(self: *Self) -> i32; }",
    );
    let _ = fixture.lower();
    fixture.expect("E2026", "declared twice");
}

#[test]
fn a_trait_receiver_must_name_self() {
    let fixture = Fixture::new("trait Shape { func area(self: i32) -> i32; }");
    let _ = fixture.lower();
    fixture.expect("E2016", "not a valid receiver for the trait `Shape`");
}

fn type_named(fixture: &Fixture, hir: &crate::Hir, name: &str) -> seal_def::DefId {
    let symbol = fixture.interner.intern(name);
    hir.definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == DefKind::Struct && data.name == symbol)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no struct named `{name}`"))
}
