//! Trait registration, contract validation, and static lookup.

use seal_def::{DefId, DefKind};
use seal_ty::{Bound, RefKind, TyKind};

use crate::{ExprKind, ReceiverKind, Res, TraitMethodLookup};

use super::support::{Fixture, assert_invariants, function, lower_clean, path_res, tail};

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

#[test]
fn a_blanket_impl_and_a_concrete_impl_overlap() {
    let fixture = Fixture::new(
        "trait Show {} struct Point { x: i32 } \
         impl<T> Show for T {} impl Show for Point {}",
    );
    let _ = fixture.lower();
    fixture.expect("E2058", "overlaps");
    fixture.expect("E2058", "`T` = `Point`");
    assert!(!fixture.codes().iter().any(|code| code == "E2021"));
}

#[test]
fn two_blanket_impls_overlap() {
    let fixture = Fixture::new("trait Show {} impl<T> Show for T {} impl<U> Show for U {}");
    let _ = fixture.lower();
    fixture.expect("E2058", "overlaps");
}

#[test]
fn disjoint_trait_arguments_do_not_overlap() {
    let (fixture, hir) = lower_clean(
        "trait Keyed<K> {} struct Pair<T, U> { first: T, second: U } \
         impl<T, U> Keyed<i32> for Pair<T, U> {} \
         impl<T, U> Keyed<bool> for Pair<T, U> {}",
    );
    assert_eq!(hir.trait_impls().len(), 2);
    assert_invariants(&fixture, &hir);
}

#[test]
fn disjoint_self_arguments_do_not_overlap() {
    let (fixture, hir) = lower_clean(
        "trait Show {} struct Pair<T> { value: T } \
         impl Show for Pair<i32> {} impl Show for Pair<bool> {}",
    );
    assert_eq!(hir.trait_impls().len(), 2);
    assert_invariants(&fixture, &hir);
}

#[test]
fn duplicate_primitive_impls_are_reported_as_duplicates() {
    let fixture = Fixture::new("trait Show {} impl Show for i32 {} impl Show for i32 {}");
    let _ = fixture.lower();
    fixture.expect("E2021", "already implemented");
    assert!(!fixture.codes().iter().any(|code| code == "E2058"));
}

#[test]
fn disjoint_inherent_impls_may_share_a_function_name() {
    let (fixture, hir) = lower_clean(
        "struct Pair<T> { value: T } \
         impl Pair<i32> { func make(value: i32) -> Pair<i32> { Pair { value: value } } } \
         impl Pair<bool> { func make(value: bool) -> Pair<bool> { Pair { value: value } } } \
         func main() -> i32 { \
             let pair = Pair::<i32>::make(1); \
             pair.value \
         }",
    );
    let pair = type_named(&fixture, &hir, "Pair");
    assert_eq!(hir.methods(pair, fixture.interner.intern("make")).len(), 2);
    assert_invariants(&fixture, &hir);
}

#[test]
fn an_associated_function_needs_arguments_when_impls_are_disjoint() {
    let fixture = Fixture::new(
        "struct Pair<T> { value: T } \
         impl Pair<i32> { func make(value: i32) -> Pair<i32> { Pair { value: value } } } \
         impl Pair<bool> { func make(value: bool) -> Pair<bool> { Pair { value: value } } } \
         func main() { let pair = Pair::make(1); }",
    );
    let _ = fixture.lower();
    fixture.expect("E2063", "more than one impl");
}

#[test]
fn an_associated_function_is_reachable_through_a_type_parameter_bound() {
    let (fixture, hir) = lower_clean(
        "trait Factory { func make() -> i32; } \
         func build<T: Factory>() -> i32 { T::make() }",
    );
    let (_, body) = function(&fixture, &hir, "build");
    let ExprKind::Call { callee, .. } = hir.expr(tail(&hir, body)).kind.clone() else {
        panic!("expected a call");
    };
    let ExprKind::Path {
        res: Res::Function(function),
        ..
    } = hir.expr(callee).kind
    else {
        panic!("expected a function path");
    };
    assert_eq!(hir.definitions().kind(function), DefKind::TraitMethod);
    assert_invariants(&fixture, &hir);
}

#[test]
fn a_default_body_is_lowered_for_its_trait_method() {
    let (fixture, hir) = lower_clean(
        "trait Shape { \
             func area(self: *Self) -> i32 { \
                 let base = 6; \
                 base \
             } \
         }",
    );
    let shape = trait_named(&fixture, &hir, "Shape");
    let area = trait_method_named(&fixture, &hir, shape, "area");
    let body = hir.body_of(area).expect("the trait method has a body");
    assert_eq!(body.owner, area);
    assert_eq!(body.params.len(), 1);
    assert_eq!(hir.definitions().get(body.params[0].def).parent, Some(area));
    assert_eq!(hir.block(body.root).statements.len(), 1);
    assert!(hir.block(body.root).tail.is_some());
    assert_invariants(&fixture, &hir);
}

#[test]
fn a_default_body_keeps_self_as_its_parameter() {
    let (fixture, hir) =
        lower_clean("trait Shape { func identity(self: *Self) -> *Self { self } }");
    let shape = trait_named(&fixture, &hir, "Shape");
    let identity = trait_method_named(&fixture, &hir, shape, "identity");
    let body = hir.body_of(identity).expect("the trait method has a body");
    let TyKind::Ref { pointee, .. } = hir.ty_ctxt().kind(body.params[0].ty) else {
        panic!("the receiver is a reference");
    };
    assert!(matches!(hir.ty_ctxt().kind(pointee), TyKind::Param(_)));
    let ret = body.ret.expect("the method returns a reference");
    let TyKind::Ref { pointee, .. } = hir.ty_ctxt().kind(ret) else {
        panic!("the return type is a reference");
    };
    assert!(matches!(hir.ty_ctxt().kind(pointee), TyKind::Param(_)));
    assert_invariants(&fixture, &hir);
}

#[test]
fn a_default_associated_function_body_is_lowered() {
    let (fixture, hir) = lower_clean("trait Factory { func make() -> i32 { 1 } }");
    let factory = trait_named(&fixture, &hir, "Factory");
    let make = trait_method_named(&fixture, &hir, factory, "make");
    let body = hir.body_of(make).expect("the trait function has a body");
    assert!(body.params.is_empty());
    assert!(body.ret.is_some());
    assert_invariants(&fixture, &hir);
}

#[test]
fn default_bodies_cover_each_receiver_form() {
    let (fixture, hir) = lower_clean(
        "trait Shape { \
             func shared(self: *Self) -> i32 { 0 } \
             func exclusive(self: *mut Self) -> i32 { 1 } \
             func value(self: Self) -> i32 { 2 } \
         }",
    );
    let shape = trait_named(&fixture, &hir, "Shape");
    for (name, expected) in [
        ("shared", RefKind::Shared),
        ("exclusive", RefKind::Exclusive),
    ] {
        let method = trait_method_named(&fixture, &hir, shape, name);
        let body = hir.body_of(method).expect("the trait method has a body");
        let TyKind::Ref { kind, .. } = hir.ty_ctxt().kind(body.params[0].ty) else {
            panic!("`{name}` takes a reference receiver");
        };
        assert_eq!(kind, expected);
    }
    let value = trait_method_named(&fixture, &hir, shape, "value");
    let body = hir.body_of(value).expect("the trait method has a body");
    assert!(matches!(
        hir.ty_ctxt().kind(body.params[0].ty),
        TyKind::Param(_)
    ));
    assert_invariants(&fixture, &hir);
}

#[test]
fn an_empty_impl_uses_every_default_method() {
    let (fixture, hir) = lower_clean(
        "struct Circle { radius: i32 } \
         trait Shape { \
             func first(self: *Self) -> i32 { 1 } \
             func second(self: *Self) -> i32 { 2 } \
         } \
         impl Shape for Circle {}",
    );
    assert_eq!(hir.trait_impls().len(), 1);
    assert_eq!(hir.bodies().len(), 2);
    assert_invariants(&fixture, &hir);
}

#[test]
fn an_override_of_a_default_method_must_match_its_signature() {
    let fixture = Fixture::new(
        "struct Circle { radius: i32 } \
         trait Shape { func area(self: *Self) -> i32 { 0 } } \
         impl Shape for Circle { func area(self: *Circle) -> i64 { (*self).radius } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2024", "return type is `i64`");
}

#[test]
fn an_impl_may_omit_a_default_method() {
    let (fixture, hir) = lower_clean(
        "struct Circle { radius: i32 } \
         trait Shape { func area(self: *Self) -> i32 { 7 } } \
         impl Shape for Circle {}",
    );
    assert_eq!(hir.trait_impls().len(), 1);
    assert_eq!(hir.bodies().len(), 1);
    assert_invariants(&fixture, &hir);
}

#[test]
fn an_impl_may_mix_default_and_required_methods() {
    let (fixture, hir) = lower_clean(
        "struct Circle { radius: i32 } \
         trait Shape { \
             func area(self: *Self) -> i32 { 7 } \
             func radius(self: *Self) -> i32; \
         } \
         impl Shape for Circle { \
             func radius(self: *Circle) -> i32 { (*self).radius } \
         }",
    );
    let circle = type_named(&fixture, &hir, "Circle");
    let TraitMethodLookup::Found {
        function: radius, ..
    } = hir.trait_method(circle, fixture.interner.intern("radius"))
    else {
        panic!("radius is implemented");
    };
    assert_eq!(hir.definitions().get(radius).parent, Some(circle));
    assert_invariants(&fixture, &hir);
}

#[test]
fn an_impl_may_override_a_default_method() {
    let (fixture, hir) = lower_clean(
        "struct Circle { radius: i32 } \
         trait Shape { func area(self: *Self) -> i32 { 7 } } \
         impl Shape for Circle { func area(self: *Circle) -> i32 { 8 } }",
    );
    let circle = type_named(&fixture, &hir, "Circle");
    let TraitMethodLookup::Found { function, .. } =
        hir.trait_method(circle, fixture.interner.intern("area"))
    else {
        panic!("area is implemented");
    };
    assert_eq!(hir.definitions().get(function).parent, Some(circle));
    assert_eq!(hir.bodies().len(), 2);
    assert_invariants(&fixture, &hir);
}

#[test]
fn a_default_body_env_proves_self_implements_the_trait() {
    let (fixture, hir) = lower_clean("trait Shape { func area(self: *Self) -> i32 { 0 } }");
    let shape = trait_named(&fixture, &hir, "Shape");
    let area = trait_method_named(&fixture, &hir, shape, "area");
    let self_param = hir
        .flattened_generics(area)
        .into_iter()
        .find(|parameter| parameter.is_self)
        .expect("the trait method inherits Self");
    let self_ty = hir.ty_ctxt().param_ty(self_param.param);
    assert!(
        hir.param_env(area).bounds(self_param.param).iter().any(
            |bound| matches!(bound, Bound::Trait(trait_ref) if trait_ref.trait_def == shape && trait_ref.self_ty == self_ty)
        ),
        "the default body must see the implicit Self: Trait bound"
    );
}

#[test]
fn a_default_body_sees_the_traits_own_parameters() {
    let (fixture, hir) =
        lower_clean("trait Convert<T> { func convert(self: *Self) -> T { self.convert() } }");
    let convert = trait_named(&fixture, &hir, "Convert");
    let method = trait_method_named(&fixture, &hir, convert, "convert");
    let body = hir.body_of(method).expect("the trait method has a body");
    let ret = body.ret.expect("convert returns T");
    assert!(matches!(hir.ty_ctxt().kind(ret), TyKind::Param(_)));
    assert_invariants(&fixture, &hir);
}

fn trait_named(fixture: &Fixture, hir: &crate::Hir, name: &str) -> DefId {
    let symbol = fixture.interner.intern(name);
    hir.definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == DefKind::Trait && data.name == symbol)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no trait named `{name}`"))
}

fn trait_method_named(fixture: &Fixture, hir: &crate::Hir, trait_def: DefId, name: &str) -> DefId {
    let symbol = fixture.interner.intern(name);
    hir.definitions()
        .iter_enumerated()
        .find(|(_, data)| {
            data.kind == DefKind::TraitMethod
                && data.parent == Some(trait_def)
                && data.name == symbol
        })
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no trait method `{name}`"))
}

fn type_named(fixture: &Fixture, hir: &crate::Hir, name: &str) -> seal_def::DefId {
    let symbol = fixture.interner.intern(name);
    hir.definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == DefKind::Struct && data.name == symbol)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no struct named `{name}`"))
}
