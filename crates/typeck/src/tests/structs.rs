//! Struct literals, field access, and initialization of projected places.

use crate::PlaceProjection;

use super::support::{
    Fixture, check_clean, check_errors, expr_statement, expr_ty, function, let_binding, statements,
    tail, ty_text_with,
};

#[test]
fn a_struct_literal_has_the_nominal_type_and_its_fields_are_checked() {
    let (fixture, hir, results) = check_clean(
        "struct Point { x: i32, y: i32 } \
         func main() -> i32 { let p = Point { x: 1, y: 2 }; p.x }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    let local = let_binding(&hir, statements(&hir, body)[0]);
    let local_ty = results.local_ty(local).expect("binding type");
    assert_eq!(ty_text_with(&fixture, &hir, local_ty), "Point");

    let field = tail(&hir, body);
    assert_eq!(
        ty_text_with(&fixture, &hir, expr_ty(&results, field)),
        "i32"
    );
    let place = results.place(field).expect("field access is a place");
    assert!(matches!(
        place.projections.as_slice(),
        [PlaceProjection::Field(_)]
    ));
    assert_eq!(
        fixture.symbol(hir.definitions().name(place.root_def())),
        "p"
    );
}

#[test]
fn field_initializers_must_match_the_declared_field_types() {
    let fixture =
        Fixture::new("struct Point { x: i32 } func main() { let p = Point { x: true }; }");
    let _ = fixture.check(64);
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn an_unknown_field_on_a_struct_is_reported() {
    let fixture =
        Fixture::new("struct Point { x: i32 } func main() -> i32 { let p = Point { x: 1 }; p.z }");
    let _ = fixture.check(64);
    fixture.expect("E2007", "no field `z`");
}

#[test]
fn a_field_of_a_non_struct_is_reported() {
    let fixture = Fixture::new("func main() -> i32 { let x = 1; x.y }");
    let _ = fixture.check(64);
    fixture.expect("E3015", "has no fields");
}

#[test]
fn field_access_through_a_reference_requires_an_explicit_deref() {
    let fixture = Fixture::new("struct Point { x: i32 } func f(p: *Point) -> i32 { p.x }");
    let _ = fixture.check(64);
    fixture.expect("E3015", "through a reference");
}

#[test]
fn an_explicit_deref_allows_field_access() {
    let (fixture, hir, results) =
        check_clean("struct Point { x: i32 } func f(p: *Point) -> i32 { (*p).x }");
    let (_, body) = function(&fixture, &hir, "f");
    let field = tail(&hir, body);
    assert_eq!(
        ty_text_with(&fixture, &hir, expr_ty(&results, field)),
        "i32"
    );
    let place = results.place(field).expect("place");
    assert!(matches!(
        place.projections.as_slice(),
        [PlaceProjection::Deref, PlaceProjection::Field(_)]
    ));
    assert!(place.contains_deref());
}

#[test]
fn reading_an_uninitialized_field_names_the_field() {
    let fixture =
        Fixture::new("struct Point { x: i32, y: i32 } func main() -> i32 { let p: Point; p.x }");
    let _ = fixture.check(64);
    fixture.expect("E3011", "`p.x` is possibly uninitialized");
}

#[test]
fn a_partially_initialized_struct_can_read_initialized_fields() {
    let (fixture, hir, results) = check_clean(
        "struct Point { x: i32, y: i32 } \
         func main() -> i32 { let mut p: Point; p.x = 1; p.x }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    let field = tail(&hir, body);
    assert_eq!(
        ty_text_with(&fixture, &hir, expr_ty(&results, field)),
        "i32"
    );
}

#[test]
fn reading_an_initialized_sibling_does_not_initialize_the_whole_struct() {
    let fixture = Fixture::new(
        "struct Point { x: i32, y: i32 } \
         func main() -> i32 { let mut p: Point; p.x = 1; p }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3011", "`p` is possibly uninitialized");
}

#[test]
fn a_whole_assignment_initializes_every_field() {
    let (_, _, _) = check_clean(
        "struct Point { x: i32, y: i32 } \
         func main() -> i32 { let mut p: Point; p = Point { x: 1, y: 2 }; p.y }",
    );
}

#[test]
fn writing_a_field_requires_a_mutable_binding() {
    let fixture = Fixture::new(
        "struct Point { x: i32 } \
         func main() -> i32 { let p = Point { x: 1 }; p.x = 2; p.x }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3004", "not declared as mutable");
}

#[test]
fn writing_a_field_of_a_mutable_binding_is_allowed() {
    let (fixture, hir, results) = check_clean(
        "struct Point { x: i32 } \
         func main() -> i32 { let mut p = Point { x: 1 }; p.x = 2; p.x }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    assert_eq!(
        ty_text_with(&fixture, &hir, expr_ty(&results, tail(&hir, body))),
        "i32"
    );
}

#[test]
fn compound_assignment_and_update_read_and_write_a_field() {
    check_clean(
        "struct Point { x: i32 } \
         func main() { let mut p = Point { x: 1 }; p.x += 2; p.x++; }",
    );

    let fixture = Fixture::new(
        "struct Point { x: i32 } \
         func main() -> i32 { let mut p: Point; p.x += 1; p.x }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3011", "`p.x` is possibly uninitialized");
}

#[test]
fn writing_through_a_reference_is_rejected() {
    let fixture = Fixture::new("struct Point { x: i32 } func f(p: *Point) { (*p).x = 1; }");
    let _ = fixture.check(64);
    fixture.expect("E3014", "shared reference");
}

#[test]
fn a_struct_literal_in_a_binding_must_match_the_annotation() {
    let fixture = Fixture::new(
        "struct Point { x: i32 } struct Other { x: i32 } \
         func main() { let p: Other = Point { x: 1 }; }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn a_reference_field_accepts_a_borrow() {
    check_clean(
        "struct Ref { value: *i32 } \
         func main() -> i32 { let x = 1; let r = Ref { value: &x }; *r.value }",
    );
}

#[test]
fn field_types_are_visible_to_calls() {
    check_clean(
        "struct Point { x: i64 } func id(v: i64) -> i64 { v } \
         func main() -> i64 { let p = Point { x: 1 }; id(p.x) }",
    );

    let fixture = Fixture::new(
        "struct Point { x: i32 } func id(v: i64) -> i64 { v } \
         func main() -> i64 { let p = Point { x: 1 }; id(p.x) }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn assignment_results_are_unit_typed() {
    let (fixture, hir, results) = check_clean(
        "struct Point { x: i32 } \
         func main() { let mut p = Point { x: 1 }; p.x = 2; }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    let statement = statements(&hir, body)[1];
    let expr = expr_statement(&hir, statement);
    assert_eq!(ty_text_with(&fixture, &hir, expr_ty(&results, expr)), "()");
}

#[test]
fn error_types_suppress_cascades_in_struct_literals() {
    let fixture =
        Fixture::new("struct Point { x: Missing } func main() { let p = Point { x: 1 }; }");
    let _ = fixture.check(64);
    fixture.expect("E2002", "cannot find type");
    assert!(
        !fixture.codes().iter().any(|code| code == "E2007"),
        "missing types must not cascade into field errors"
    );
    let _ = check_errors("struct Point { x: i32 } func main() { let p = Point { x: 1 }; }");
}
