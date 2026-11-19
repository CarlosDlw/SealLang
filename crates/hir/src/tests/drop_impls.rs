//! `Drop` installation, implementation validation, and destructor lookup.

use seal_def::DefKind;

use super::support::{Fixture, lower_clean};
use crate::Hir;

const POINT: &str = "\
struct Point { x: i32, y: i32 } \
impl Drop for Point { \
    func drop(self: *mut Point) { (*self).x = 0; } \
}";

/// Returns the aggregate definition named `name`.
fn aggregate(fixture: &Fixture, hir: &Hir, name: &str) -> seal_ty::AdtId {
    let definition = hir
        .definitions()
        .iter_enumerated()
        .find(|(_, data)| {
            matches!(data.kind, DefKind::Struct | DefKind::Enum)
                && fixture.symbol(data.name) == name
        })
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no aggregate named `{name}`"));
    hir.adt(definition).expect("the aggregate is reserved")
}

#[test]
fn the_drop_trait_is_installed_in_every_hir() {
    let (fixture, hir) = lower_clean("");
    let trait_def = hir.drop_trait();
    let data = hir.definitions().get(trait_def);
    assert_eq!(data.kind, DefKind::Trait);
    assert_eq!(fixture.symbol(data.name), "Drop");
    let declaration = hir.trait_definition(trait_def).expect("Drop is declared");
    assert_eq!(declaration.methods.len(), 1);
    let method = declaration.methods[0].clone();
    assert_eq!(fixture.symbol(method.name), "drop");
    assert_eq!(method.def, hir.drop_method());
    assert!(!method.has_body, "Drop declares no default body");
    assert_eq!(
        method.receiver.map(|receiver| receiver.reference),
        Some(Some(seal_ty::RefKind::Exclusive)),
        "the destructor takes an exclusive raw pointer"
    );
    assert!(declaration.supertraits.is_empty());
}

#[test]
fn a_valid_implementation_records_the_destructor() {
    let (fixture, hir) = lower_clean(POINT);
    let point = aggregate(&fixture, &hir, "Point");
    let recorded = hir
        .ty_ctxt()
        .drop_method(point)
        .expect("a validated implementation records its destructor");
    let data = hir.definitions().get(recorded);
    assert_eq!(data.kind, DefKind::Function);
    assert_eq!(fixture.symbol(data.name), "drop");
    assert!(data.parent.is_some(), "the destructor belongs to an impl");
}

#[test]
fn a_generic_implementation_covers_the_whole_declaration() {
    let source = "\
struct Box<T> { value: T } \
impl<T> Drop for Box<T> { func drop(self: *mut Box<T>) {} }";
    let (fixture, hir) = lower_clean(source);
    let boxed = aggregate(&fixture, &hir, "Box");
    assert!(hir.ty_ctxt().drop_method(boxed).is_some());
}

#[test]
fn a_second_implementation_is_rejected() {
    let source = "\
struct Point { x: i32 } \
impl Drop for Point { func drop(self: *mut Point) {} } \
impl Drop for Point { func drop(self: *mut Point) {} }";
    let fixture = Fixture::new(source);
    let _ = fixture.lower();
    fixture.expect("E3067", "implements `Drop` more than once");
}

#[test]
fn an_implementation_with_reordered_parameters_is_rejected() {
    let source = "\
struct Pair<A, B> { first: A, second: B } \
impl<T, U> Drop for Pair<U, T> { func drop(self: *mut Pair<U, T>) {} }";
    let fixture = Fixture::new(source);
    let _ = fixture.lower();
    fixture.expect("E3067", "does not cover the whole type declaration");
}

#[test]
fn a_specialized_implementation_is_rejected() {
    let source = "\
struct Box<T> { value: T } \
impl Drop for Box<i32> { func drop(self: *mut Box<i32>) {} }";
    let fixture = Fixture::new(source);
    let _ = fixture.lower();
    fixture.expect("E3067", "does not cover the whole type declaration");
}

#[test]
fn an_implementation_for_a_non_nominal_type_is_rejected() {
    for source in [
        "impl Drop for i32 { func drop(self: *mut i32) {} }",
        "impl Drop for *i32 { func drop(self: *mut *i32) {} }",
        "impl Drop for (i32, i32) { func drop(self: *mut (i32, i32)) {} }",
    ] {
        let fixture = Fixture::new(source);
        let _ = fixture.lower();
        fixture.expect("E2015", "");
    }
}

#[test]
fn drop_cannot_be_a_trait_bound() {
    let fixture = Fixture::new("func f<T: Drop>() {}");
    let _ = fixture.lower();
    fixture.expect("E3064", "cannot be used as a trait bound");
}

#[test]
fn drop_cannot_be_a_supertrait() {
    let fixture = Fixture::new("trait Named: Drop {}");
    let _ = fixture.lower();
    fixture.expect("E3064", "cannot be used as a trait bound");
}

#[test]
fn drop_cannot_be_a_trait_object() {
    let fixture = Fixture::new("func f(value: *dyn Drop) {}");
    let _ = fixture.lower();
    fixture.expect("E3066", "cannot be used as a trait object");
}

#[test]
fn the_name_drop_is_reserved() {
    let fixture = Fixture::new("trait Drop {}");
    let _ = fixture.lower();
    fixture.expect("E2066", "reserved");
}

#[test]
fn an_implementation_without_the_destructor_is_rejected() {
    let fixture = Fixture::new("struct Point { x: i32 } impl Drop for Point {}");
    let _ = fixture.lower();
    fixture.expect("E2022", "does not implement");
}

#[test]
fn a_destructor_with_the_wrong_receiver_is_rejected() {
    let source = "\
struct Point { x: i32 } \
impl Drop for Point { func drop(self: *Point) {} }";
    let fixture = Fixture::new(source);
    let _ = fixture.lower();
    fixture.expect("E2024", "does not match");
}

#[test]
fn a_value_receiver_is_rejected() {
    let source = "\
struct Point { x: i32 } \
impl Drop for Point { func drop(self: Point) {} }";
    let fixture = Fixture::new(source);
    let _ = fixture.lower();
    fixture.expect("E2024", "does not match");
}

#[test]
fn a_shared_receiver_is_rejected() {
    let source = "\
struct Point { x: i32 } \
impl Drop for Point { func drop(self: *Point) {} }";
    let fixture = Fixture::new(source);
    let _ = fixture.lower();
    fixture.expect("E2024", "does not match");
}

#[test]
fn an_extra_function_is_rejected() {
    let source = "\
struct Point { x: i32 } \
impl Drop for Point { \
    func drop(self: *mut Point) {} \
    func tidy(self: *mut Point) {} \
}";
    let fixture = Fixture::new(source);
    let _ = fixture.lower();
    fixture.expect("E2023", "does not declare");
}

#[test]
fn a_destructor_with_the_wrong_return_type_is_rejected() {
    let source = "\
struct Point { x: i32 } \
impl Drop for Point { func drop(self: *mut Point) -> i32 { 0 } }";
    let fixture = Fixture::new(source);
    let _ = fixture.lower();
    fixture.expect("E2024", "does not match");
}
