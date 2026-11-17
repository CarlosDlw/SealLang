//! Trait-object lowering and dyn-compatibility validation.

use seal_def::{DefId, DefKind};
use seal_ty::{IntTy, TyKind};

use super::support::{Fixture, assert_invariants, function, lower_clean};

fn named(fixture: &Fixture, hir: &crate::Hir, name: &str, kind: DefKind) -> DefId {
    let symbol = fixture.interner.intern(name);
    hir.definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == kind && data.name == symbol)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no {kind:?} named `{name}`"))
}

fn object_type(hir: &crate::Hir, ty: seal_ty::TyId) -> (DefId, Vec<seal_ty::TyId>) {
    let TyKind::Ref { pointee, .. } = hir.ty_ctxt().kind(ty) else {
        panic!("expected a pointer to a trait object");
    };
    let TyKind::Dyn { trait_def, args } = hir.ty_ctxt().kind(pointee) else {
        panic!(
            "expected a trait object, got {:?}",
            hir.ty_ctxt().kind(pointee)
        );
    };
    (trait_def, hir.ty_ctxt().substs_args(args))
}

fn count(fixture: &Fixture, code: &str) -> usize {
    fixture
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code().map(|code| code.as_str()) == Some(code))
        .count()
}

fn note_messages(fixture: &Fixture, code: &str) -> Vec<String> {
    fixture
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code().map(|code| code.as_str()) == Some(code))
        .flat_map(|diagnostic| {
            diagnostic
                .labels()
                .iter()
                .map(|label| label.message.clone())
        })
        .collect()
}

#[test]
fn dyn_type_lowers_to_the_principal_trait_with_arguments() {
    let (fixture, hir) = lower_clean(
        "trait Convert<T> { func convert(self: *Self) -> T; } func f(value: *dyn Convert<i32>) {}",
    );
    let convert = named(&fixture, &hir, "Convert", DefKind::Trait);
    let (_, body) = function(&fixture, &hir, "f");
    let (trait_def, arguments) = object_type(&hir, body.params[0].ty);
    assert_eq!(trait_def, convert);
    assert_eq!(hir.ty_ctxt().kind(arguments[0]), TyKind::Int(IntTy::I32));
    assert_eq!(hir.dyn_types().len(), 1);
    assert_invariants(&fixture, &hir);
}

#[test]
fn object_safe_trait_forms_a_trait_object() {
    let (fixture, hir) = lower_clean(
        "trait Shape { func area(self: *Self) -> i32; func resize(self: *mut Self, factor: i32); } \
         func f(value: *dyn Shape) {}",
    );
    assert_eq!(hir.dyn_types().len(), 1);
    assert_invariants(&fixture, &hir);
}

#[test]
fn empty_trait_is_object_safe() {
    lower_clean("trait Marker {} func f(value: *dyn Marker) {}");
}

#[test]
fn exclusive_receiver_is_object_safe() {
    lower_clean("trait Bump { func bump(self: *mut Self); } func f(value: *dyn Bump) {}");
}

#[test]
fn by_value_receiver_is_not_object_safe() {
    let fixture =
        Fixture::new("trait Bad { func consume(self: Self); } func f(value: *dyn Bad) {}");
    let _ = fixture.lower();
    fixture.expect("E2065", "cannot be used as a trait object");
    let notes = note_messages(&fixture, "E2065");
    assert!(
        notes
            .iter()
            .any(|note| note.contains("consume") && note.contains("by value")),
        "unexpected notes: {notes:?}"
    );
    assert_eq!(count(&fixture, "E2065"), 1);
}

#[test]
fn static_associated_function_is_not_object_safe() {
    let fixture = Fixture::new("trait Bad { func make() -> i32; } func f(value: *dyn Bad) {}");
    let _ = fixture.lower();
    let notes = note_messages(&fixture, "E2065");
    assert!(
        notes
            .iter()
            .any(|note| note.contains("make") && note.contains("no receiver")),
        "unexpected notes: {notes:?}"
    );
}

#[test]
fn generic_method_is_not_object_safe() {
    let fixture = Fixture::new(
        "trait Bad { func map<T>(self: *Self, value: T); } func f(value: *dyn Bad) {}",
    );
    let _ = fixture.lower();
    let notes = note_messages(&fixture, "E2065");
    assert!(
        notes
            .iter()
            .any(|note| note.contains("map") && note.contains("type parameters")),
        "unexpected notes: {notes:?}"
    );
}

#[test]
fn self_in_a_parameter_is_not_object_safe() {
    let fixture = Fixture::new(
        "trait Bad { func merge(self: *Self, other: *Self); } func f(value: *dyn Bad) {}",
    );
    let _ = fixture.lower();
    let notes = note_messages(&fixture, "E2065");
    assert!(
        notes
            .iter()
            .any(|note| note.contains("merge") && note.contains("outside its receiver")),
        "unexpected notes: {notes:?}"
    );
}

#[test]
fn self_nested_in_a_parameter_type_is_not_object_safe() {
    let fixture = Fixture::new(
        "trait Bad { func pair(self: *Self, other: (Self, i32)); } func f(value: *dyn Bad) {}",
    );
    let _ = fixture.lower();
    let notes = note_messages(&fixture, "E2065");
    assert!(
        notes
            .iter()
            .any(|note| note.contains("pair") && note.contains("outside its receiver")),
        "unexpected notes: {notes:?}"
    );
}

#[test]
fn self_in_the_return_type_is_not_object_safe() {
    let fixture =
        Fixture::new("trait Bad { func clone(self: *Self) -> Self; } func f(value: *dyn Bad) {}");
    let _ = fixture.lower();
    let notes = note_messages(&fixture, "E2065");
    assert!(
        notes
            .iter()
            .any(|note| note.contains("clone") && note.contains("outside its receiver")),
        "unexpected notes: {notes:?}"
    );
}

#[test]
fn self_behind_a_pointer_in_a_parameter_is_not_object_safe() {
    let fixture = Fixture::new(
        "trait Bad { func inspect(self: *Self, other: **Self); } func f(value: *dyn Bad) {}",
    );
    let _ = fixture.lower();
    let notes = note_messages(&fixture, "E2065");
    assert!(
        notes
            .iter()
            .any(|note| note.contains("inspect") && note.contains("outside its receiver")),
        "unexpected notes: {notes:?}"
    );
}

#[test]
fn object_safety_diagnostic_names_the_offending_method() {
    let fixture = Fixture::new(
        "trait Bad { func merge(self: *Self, other: *Self); } func f(value: *dyn Bad) {}",
    );
    let _ = fixture.lower();
    let notes = note_messages(&fixture, "E2065");
    assert!(
        notes.iter().any(|note| note.contains("merge")),
        "unexpected notes: {notes:?}"
    );
}

#[test]
fn supertrait_chain_is_checked_transitively() {
    let fixture = Fixture::new(
        "trait Root { func consume(self: Self); } trait Middle: Root {} trait Leaf: Middle {} \
         func f(value: *dyn Leaf) {}",
    );
    let _ = fixture.lower();
    fixture.expect("E2065", "Leaf");
    let notes = note_messages(&fixture, "E2065");
    assert!(
        notes.iter().any(|note| note.contains("consume")),
        "unexpected notes: {notes:?}"
    );
}

#[test]
fn cyclic_supertraits_do_not_recurse_forever() {
    let fixture = Fixture::new("trait A: B {} trait B: A {} func f(value: *dyn A) {}");
    let _ = fixture.lower();
    fixture.expect("E2064", "cyclic");
    assert_eq!(count(&fixture, "E2065"), 0);
}

#[test]
fn non_object_safe_supertrait_is_rejected() {
    let fixture = Fixture::new(
        "trait Base { func consume(self: Self); } trait Child: Base {} \
         func f(value: *dyn Child) {}",
    );
    let _ = fixture.lower();
    fixture.expect("E2065", "Child");
    let notes = note_messages(&fixture, "E2065");
    assert!(
        notes.iter().any(|note| note.contains("consume")),
        "unexpected notes: {notes:?}"
    );
}

#[test]
fn diamond_object_safety_notes_are_not_duplicated() {
    let fixture = Fixture::new(
        "trait Base { func consume(self: Self); } \
         trait Left: Base {} trait Right: Base {} trait Diamond: Left + Right {} \
         func f(value: *dyn Diamond) {}",
    );
    let _ = fixture.lower();
    assert_eq!(count(&fixture, "E2065"), 1);
    let duplicated = note_messages(&fixture, "E2065")
        .iter()
        .filter(|note| note.contains("consume"))
        .count();
    assert_eq!(duplicated, 1);
}

#[test]
fn self_in_a_supertrait_argument_is_rejected() {
    let fixture =
        Fixture::new("trait Base<T> {} trait Child: Base<Self> {} func f(value: *dyn Child) {}");
    let _ = fixture.lower();
    fixture.expect("E2065", "Child");
    let notes = note_messages(&fixture, "E2065");
    assert!(
        notes
            .iter()
            .any(|note| note.contains("Base") && note.contains("arguments")),
        "unexpected notes: {notes:?}"
    );
}

#[test]
fn generic_trait_with_concrete_arguments_is_object_safe() {
    lower_clean(
        "trait Convert<T> { func convert(self: *Self) -> T; } func f(value: *dyn Convert<i32>) {}",
    );
}

#[test]
fn dyn_of_an_unknown_trait_is_reported_once() {
    let fixture = Fixture::new("func f(value: *dyn Missing) {}");
    let _ = fixture.lower();
    assert_eq!(count(&fixture, "E2057"), 1);
    assert_eq!(count(&fixture, "E2065"), 0);
}

#[test]
fn dyn_of_a_struct_is_reported_once() {
    let fixture = Fixture::new("struct Point { x: i32 } func f(value: *dyn Point) {}");
    let _ = fixture.lower();
    assert_eq!(count(&fixture, "E2060"), 1);
    assert_eq!(count(&fixture, "E2065"), 0);
}

#[test]
fn dyn_with_wrong_trait_arguments_is_reported_once() {
    let fixture = Fixture::new("trait Convert<T> {} func f(value: *dyn Convert) {}");
    let _ = fixture.lower();
    assert_eq!(count(&fixture, "E2052"), 1);
    assert_eq!(count(&fixture, "E2065"), 0);
}

#[test]
fn impl_target_cannot_be_dyn() {
    let fixture = Fixture::new("trait Shape {} impl Shape for dyn Shape {}");
    let _ = fixture.lower();
    assert_eq!(count(&fixture, "E2056"), 1);
    assert_eq!(count(&fixture, "E2027"), 0);
    assert_eq!(count(&fixture, "E2028"), 0);
}

#[test]
fn static_use_of_a_non_object_safe_trait_still_works() {
    lower_clean(
        "trait Bad { func make() -> Self; } struct Point { x: i32 } \
         impl Bad for Point { func make() -> Point { Point { x: 1 } } } \
         func f() -> Point { Point::make() }",
    );
}

#[test]
fn dyn_types_are_recorded_per_use() {
    let (fixture, hir) = lower_clean("trait Shape {} func f(a: *dyn Shape, b: *dyn Shape) {}");
    assert_eq!(hir.dyn_types().len(), 2);
    assert_eq!(hir.dyn_types()[0].ty, hir.dyn_types()[1].ty);
    assert_invariants(&fixture, &hir);
}

#[test]
fn nested_dyn_in_tuple_and_array_is_recorded() {
    let (fixture, hir) =
        lower_clean("trait Shape {} func f(value: (dyn Shape, [dyn Shape; 2])) {}");
    assert_eq!(hir.dyn_types().len(), 2);
    assert_invariants(&fixture, &hir);
}

#[test]
fn object_safety_violation_is_reported_once_per_dyn_use() {
    let fixture =
        Fixture::new("trait Bad { func consume(self: Self); } func f(a: *dyn Bad, b: *dyn Bad) {}");
    let _ = fixture.lower();
    assert_eq!(count(&fixture, "E2065"), 2);
}
