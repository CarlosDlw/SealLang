//! Opt-in `Copy` installation, validation, and registry tests.

use seal_def::DefKind;

use super::support::{Fixture, lower_clean};
use crate::Hir;

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

fn has_code(fixture: &Fixture, code: &str) -> bool {
    fixture.codes().iter().any(|candidate| candidate == code)
}

fn labels(fixture: &Fixture, code: &str) -> Vec<String> {
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

fn count(fixture: &Fixture, code: &str) -> usize {
    fixture
        .codes()
        .iter()
        .filter(|candidate| candidate.as_str() == code)
        .count()
}

fn helps(fixture: &Fixture, code: &str) -> Vec<String> {
    fixture
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code().map(|code| code.as_str()) == Some(code))
        .flat_map(|diagnostic| diagnostic.helps().iter().cloned())
        .collect()
}

#[test]
fn copy_is_installed_as_an_empty_builtin_trait() {
    let (fixture, hir) = lower_clean("");
    let copy = hir.copy_trait();

    assert_eq!(hir.definitions().get(copy).kind, DefKind::Trait);
    assert_eq!(fixture.symbol(hir.definitions().get(copy).name), "Copy");
    assert!(hir.is_copy_trait(copy));
    assert!(hir.is_builtin_def(copy));

    let declaration = hir.trait_definition(copy).expect("Copy is declared");
    assert!(declaration.methods.is_empty());
    assert!(declaration.supertraits.is_empty());
    assert!(declaration.generics.params[0].is_self);
    assert!(hir.is_builtin_def(declaration.generics.params[0].def));
}

#[test]
fn a_valid_struct_implementation_records_the_impl() {
    let (fixture, hir) = lower_clean("struct Point { x: i32, y: i32 } impl Copy for Point {}");
    let point = aggregate(&fixture, &hir, "Point");
    let recorded = hir
        .ty_ctxt()
        .copy_impl(point)
        .expect("a valid implementation is registered");
    assert_eq!(hir.definitions().get(recorded).kind, DefKind::Impl);
}

#[test]
fn a_valid_enum_implementation_records_the_impl() {
    let (fixture, hir) = lower_clean("enum Shape { Empty, Holds(i32) } impl Copy for Shape {}");
    let shape = aggregate(&fixture, &hir, "Shape");
    assert!(hir.ty_ctxt().copy_impl(shape).is_some());
}

#[test]
fn a_generic_implementation_with_a_copy_bound_is_accepted() {
    let (fixture, hir) = lower_clean("struct Box<T> { value: T } impl<T: Copy> Copy for Box<T> {}");
    let boxed = aggregate(&fixture, &hir, "Box");
    assert!(hir.ty_ctxt().copy_impl(boxed).is_some());
}

#[test]
fn a_generic_parameter_without_a_bound_is_rejected() {
    let fixture = Fixture::new("struct Box<T> { value: T } impl<T> Copy for Box<T> {}");
    let hir = fixture.lower();
    let boxed = aggregate(&fixture, &hir, "Box");

    fixture.expect("E3077", "field `value`");
    assert_eq!(count(&fixture, "E3077"), 1, "one impl reports one field");
    assert!(
        fixture
            .diagnostics()
            .iter()
            .any(
                |diagnostic| diagnostic.code().map(|code| code.as_str()) == Some("E3077")
                    && diagnostic.message().contains("`T`")
            ),
        "the message shows the non-Copy parameter type"
    );
    assert!(
        helps(&fixture, "E3077")
            .iter()
            .any(|help| help.contains("add `T: Copy`")),
        "the help names the missing bound"
    );
    assert!(hir.ty_ctxt().copy_impl(boxed).is_none());
}

#[test]
fn a_raw_pointer_field_needs_no_bound_on_its_pointee() {
    let (fixture, hir) = lower_clean("struct Ptr<T> { value: *raw T } impl<T> Copy for Ptr<T> {}");
    let ptr = aggregate(&fixture, &hir, "Ptr");
    assert!(hir.ty_ctxt().copy_impl(ptr).is_some());
}

#[test]
fn reference_fields_need_no_bound_on_their_pointee() {
    for field in ["*T", "*mut T"] {
        let source = format!("struct Ptr<T> {{ value: {field} }} impl<T> Copy for Ptr<T> {{}}");
        let (fixture, hir) = lower_clean(&source);
        let ptr = aggregate(&fixture, &hir, "Ptr");
        assert!(hir.ty_ctxt().copy_impl(ptr).is_some(), "field type {field}");
    }
}

#[test]
fn a_specialized_implementation_is_rejected() {
    let fixture = Fixture::new("struct Box<T> { value: T } impl Copy for Box<i32> {}");
    let _ = fixture.lower();
    fixture.expect("E3076", "does not cover the whole type declaration");
}

#[test]
fn an_identical_duplicate_is_left_to_coherence() {
    let fixture =
        Fixture::new("struct Point { x: i32 } impl Copy for Point {} impl Copy for Point {}");
    let _ = fixture.lower();

    fixture.expect("E2021", "already implemented");
    assert!(!has_code(&fixture, "E3076"));
}

#[test]
fn an_identical_generic_duplicate_is_left_to_coherence() {
    let fixture = Fixture::new(
        "struct Box<T> { value: T } \
         impl<T: Copy> Copy for Box<T> {} \
         impl<T: Copy> Copy for Box<T> {}",
    );
    let _ = fixture.lower();

    fixture.expect("E2058", "overlaps");
    assert!(!has_code(&fixture, "E3076"));
}

#[test]
fn reordered_parameters_are_rejected() {
    let fixture = Fixture::new(
        "struct Pair<A, B> { first: A, second: B } \
         impl<T, U> Copy for Pair<U, T> {}",
    );
    let _ = fixture.lower();
    fixture.expect("E3076", "does not cover the whole type declaration");
}

#[test]
fn an_extra_parameter_is_rejected() {
    let fixture = Fixture::new("struct Box<T> { value: T } impl<T, U> Copy for Box<T> {}");
    let _ = fixture.lower();
    fixture.expect("E3076", "does not cover the whole type declaration");
}

#[test]
fn a_non_nominal_target_is_rejected() {
    for source in [
        "impl Copy for i32 {}",
        "impl Copy for *i32 {}",
        "impl Copy for *raw i32 {}",
        "impl Copy for *raw mut i32 {}",
        "impl Copy for (i32, i32) {}",
        "impl Copy for [i32; 3] {}",
        "impl Copy for str {}",
        "impl Copy for [i32] {}",
        "trait Show {} impl Copy for dyn Show {}",
    ] {
        let fixture = Fixture::new(source);
        let _ = fixture.lower();
        fixture.expect("E2015", "not a nominal type");
    }
}

#[test]
fn a_function_in_a_copy_impl_is_rejected() {
    let fixture =
        Fixture::new("struct Point { x: i32 } impl Copy for Point { func extra(self: *Point) {} }");
    let _ = fixture.lower();
    fixture.expect("E2023", "does not declare `extra`");
}

#[test]
fn copy_and_drop_are_mutually_exclusive_in_both_orders() {
    let sources = [
        "struct Point { x: i32 } \
         impl Copy for Point {} \
         impl Drop for Point { func drop(self: *mut Point) {} }",
        "struct Point { x: i32 } \
         impl Drop for Point { func drop(self: *mut Point) {} } \
         impl Copy for Point {}",
    ];
    for source in sources {
        let fixture = Fixture::new(source);
        let hir = fixture.lower();
        let point = aggregate(&fixture, &hir, "Point");

        fixture.expect("E3068", "cannot implement `Copy`");
        assert!(
            labels(&fixture, "E3068")
                .iter()
                .any(|label| label.contains("destructor")),
            "the conflict points at the Drop implementation"
        );
        assert!(hir.ty_ctxt().copy_impl(point).is_none());
        assert!(
            hir.ty_ctxt().drop_method(point).is_some(),
            "Drop remains registered"
        );
    }
}

#[test]
fn the_name_copy_is_reserved() {
    let fixture = Fixture::new("trait Copy {}");
    let _ = fixture.lower();
    fixture.expect("E2066", "reserved");
}

#[test]
fn copy_cannot_be_a_trait_object() {
    let fixture = Fixture::new("func f(value: *dyn Copy) {}");
    let _ = fixture.lower();
    fixture.expect("E3078", "cannot be used as a trait object");
}

#[test]
fn copy_cannot_be_a_supertrait() {
    let fixture = Fixture::new("trait Named: Copy {}");
    let hir = fixture.lower();
    let named = hir
        .trait_def(fixture.interner.intern("Named"))
        .expect("the trait exists");
    let declaration = hir.trait_definition(named).expect("the trait is declared");

    assert!(declaration.supertraits.is_empty());
    fixture.expect("E3078", "cannot be a supertrait");
}

#[test]
fn a_type_parameter_cannot_implement_copy() {
    let fixture = Fixture::new("impl<T> Copy for T {}");
    let _ = fixture.lower();
    fixture.expect("E2015", "not a nominal type");
}

#[test]
fn a_valid_where_clause_is_accepted() {
    let (fixture, hir) =
        lower_clean("struct Box<T> { value: T } impl<T> Copy for Box<T> where T: Copy {}");
    let boxed = aggregate(&fixture, &hir, "Box");
    assert!(hir.ty_ctxt().copy_impl(boxed).is_some());
}

#[test]
fn validation_is_independent_of_declaration_order() {
    let (fixture, hir) = lower_clean(
        "struct Outer { inner: Inner } \
         struct Inner { value: i32 } \
         impl Copy for Outer {} \
         impl Copy for Inner {}",
    );
    let outer = aggregate(&fixture, &hir, "Outer");
    let inner = aggregate(&fixture, &hir, "Inner");
    assert!(hir.ty_ctxt().copy_impl(outer).is_some());
    assert!(hir.ty_ctxt().copy_impl(inner).is_some());
}

#[test]
fn an_invalid_inner_makes_the_outer_fail_too() {
    let fixture = Fixture::new(
        "struct Outer { inner: Inner } \
         struct Inner { marker: Marker } \
         struct Marker { value: i32 } \
         impl Copy for Outer {} \
         impl Copy for Inner {}",
    );
    let hir = fixture.lower();
    let outer = aggregate(&fixture, &hir, "Outer");
    let inner = aggregate(&fixture, &hir, "Inner");

    fixture.expect("E3077", "field `inner`");
    fixture.expect("E3077", "field `marker`");
    assert!(hir.ty_ctxt().copy_impl(outer).is_none());
    assert!(hir.ty_ctxt().copy_impl(inner).is_none());
}

#[test]
fn an_empty_struct_is_copy() {
    let (fixture, hir) = lower_clean("struct Empty {} impl Copy for Empty {}");
    let empty = aggregate(&fixture, &hir, "Empty");
    assert!(hir.ty_ctxt().copy_impl(empty).is_some());
}

#[test]
fn an_enum_with_a_non_copy_variant_is_rejected() {
    let fixture = Fixture::new(
        "struct Marker { value: i32 } \
         enum Shape { Empty, Holds(Marker) } \
         impl Copy for Shape {}",
    );
    let hir = fixture.lower();
    let shape = aggregate(&fixture, &hir, "Shape");

    fixture.expect("E3077", "variant `Holds`");
    assert!(hir.ty_ctxt().copy_impl(shape).is_none());
}

#[test]
fn recursion_through_a_raw_pointer_is_copy() {
    let (fixture, hir) = lower_clean("struct Node { next: *raw Node } impl Copy for Node {}");
    let node = aggregate(&fixture, &hir, "Node");
    assert!(hir.ty_ctxt().copy_impl(node).is_some());
}

#[test]
fn copy_has_no_associated_items_to_call() {
    let fixture = Fixture::new("func main() { Copy::algo(); }");
    let _ = fixture.lower();
    fixture.expect("E2018", "Copy");
}

#[test]
fn a_field_type_already_registered_as_copy_is_accepted() {
    let (fixture, hir) = lower_clean(
        "struct Inner { value: i32 } \
         struct Outer { inner: Inner } \
         impl Copy for Inner {} \
         impl Copy for Outer {}",
    );
    let inner = aggregate(&fixture, &hir, "Inner");
    let outer = aggregate(&fixture, &hir, "Outer");
    assert!(hir.ty_ctxt().copy_impl(inner).is_some());
    assert!(hir.ty_ctxt().copy_impl(outer).is_some());
}

#[test]
fn a_specialized_impl_next_to_a_generic_impl_reports_one_specialization() {
    let fixture = Fixture::new(
        "struct Box<T> { value: T } \
         impl Copy for Box<i32> {} \
         impl<T: Copy> Copy for Box<T> {}",
    );
    let _ = fixture.lower();
    fixture.expect("E3076", "does not cover the whole type declaration");
    assert_eq!(count(&fixture, "E3076"), 1);
}

#[test]
fn two_specialized_impls_are_both_rejected() {
    let fixture = Fixture::new(
        "struct Box<T> { value: T } \
         impl Copy for Box<i32> {} \
         impl Copy for Box<bool> {}",
    );
    let _ = fixture.lower();
    assert_eq!(count(&fixture, "E3076"), 2);
}

#[test]
fn a_repeated_type_argument_with_one_impl_parameter_is_rejected() {
    let fixture = Fixture::new(
        "struct Pair<T, U> { first: T, second: U } \
         impl<T> Copy for Pair<T, T> {}",
    );
    let _ = fixture.lower();
    fixture.expect("E3076", "does not cover the whole type declaration");
}

#[test]
fn an_array_of_non_copy_values_is_rejected() {
    let fixture = Fixture::new(
        "struct Token { value: i32 } \
         struct BadArray { values: [Token; 2] } \
         impl Copy for BadArray {}",
    );
    let _ = fixture.lower();
    fixture.expect("E3077", "field `values`");
    assert!(
        fixture
            .diagnostics()
            .iter()
            .any(
                |diagnostic| diagnostic.code().map(|code| code.as_str()) == Some("E3077")
                    && diagnostic.message().contains("[Token; 2]")
            ),
        "the message shows the array field type"
    );
}

#[test]
fn a_tuple_of_non_copy_values_is_rejected() {
    let fixture = Fixture::new(
        "struct Token { value: i32 } \
         struct BadTuple { pair: (Token, i32) } \
         impl Copy for BadTuple {}",
    );
    let _ = fixture.lower();
    fixture.expect("E3077", "field `pair`");
    assert!(
        fixture
            .diagnostics()
            .iter()
            .any(
                |diagnostic| diagnostic.code().map(|code| code.as_str()) == Some("E3077")
                    && diagnostic.message().contains("(Token, i32)")
            ),
        "the message shows the tuple field type"
    );
}

#[test]
fn a_named_enum_field_names_its_variant_and_field() {
    let fixture = Fixture::new(
        "struct Token { value: i32 } \
         enum Shape { Empty, Holds { token: Token } } \
         impl Copy for Shape {}",
    );
    let _ = fixture.lower();
    fixture.expect("E3077", "field `token` of variant `Holds`");
    assert_eq!(count(&fixture, "E3077"), 1);
}

#[test]
fn an_implementation_is_accepted_after_a_transitive_chain() {
    let (fixture, hir) = lower_clean(
        "struct Outer { middle: Middle } \
         struct Middle { inner: Inner } \
         struct Inner { value: i32 } \
         impl Copy for Outer {} \
         impl Copy for Middle {} \
         impl Copy for Inner {}",
    );
    for name in ["Outer", "Middle", "Inner"] {
        let adt = aggregate(&fixture, &hir, name);
        assert!(
            hir.ty_ctxt().copy_impl(adt).is_some(),
            "`{name}` must be registered"
        );
    }
}

#[test]
fn an_invalid_transitive_chain_reports_every_implementation_once() {
    let fixture = Fixture::new(
        "struct Marker { value: i32 } \
         struct Inner { marker: Marker } \
         struct Middle { inner: Inner } \
         struct Outer { middle: Middle } \
         impl Copy for Outer {} \
         impl Copy for Middle {} \
         impl Copy for Inner {}",
    );
    let hir = fixture.lower();
    fixture.expect("E3077", "field `middle`");
    fixture.expect("E3077", "field `inner`");
    fixture.expect("E3077", "field `marker`");
    assert_eq!(count(&fixture, "E3077"), 3);
    for name in ["Outer", "Middle", "Inner"] {
        let adt = aggregate(&fixture, &hir, name);
        assert!(
            hir.ty_ctxt().copy_impl(adt).is_none(),
            "`{name}` must not be registered"
        );
    }
}

#[test]
fn validation_is_independent_of_the_reverse_source_order() {
    let (fixture, hir) = lower_clean(
        "impl Copy for Outer {} \
         impl Copy for Inner {} \
         struct Outer { inner: Inner } \
         struct Inner { value: i32 }",
    );
    let outer = aggregate(&fixture, &hir, "Outer");
    let inner = aggregate(&fixture, &hir, "Inner");
    assert!(hir.ty_ctxt().copy_impl(outer).is_some());
    assert!(hir.ty_ctxt().copy_impl(inner).is_some());
}

#[test]
fn a_recursive_by_value_type_still_reports_e3016_with_a_copy_impl() {
    let fixture = Fixture::new("struct Node { next: Node } impl Copy for Node {}");
    let _ = fixture.lower();
    fixture.expect("E3016", "recursive");
}

#[test]
fn an_extra_impl_parameter_is_rejected_for_a_pair() {
    let fixture = Fixture::new(
        "struct Pair<T> { value: T } \
         impl<T, U> Copy for Pair<T> {}",
    );
    let _ = fixture.lower();
    fixture.expect("E3076", "whole type declaration");
}

#[test]
fn an_associated_function_in_a_copy_impl_is_rejected() {
    let fixture = Fixture::new("struct Point { x: i32 } impl Copy for Point { func extra() {} }");
    let _ = fixture.lower();
    fixture.expect("E2023", "does not declare `extra`");
}
