//! Enum construction and method calls on enum values.

use super::support::{
    Fixture, check_clean, expr_ty, function, let_binding, statements, tail, ty_text_with,
};

#[test]
fn a_variant_has_the_enum_type() {
    let (fixture, hir, results) = check_clean(
        "enum Shape { Empty, Circle { radius: i32 } } \
         func main() -> Shape { let empty = Shape::Empty; Shape::Circle { radius: 1 } }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    let local = let_binding(&hir, statements(&hir, body)[0]);
    let local_ty = results.local_ty(local).expect("binding type");
    assert_eq!(ty_text_with(&fixture, &hir, local_ty), "Shape");
    assert_eq!(
        ty_text_with(&fixture, &hir, expr_ty(&results, tail(&hir, body))),
        "Shape"
    );
}

#[test]
fn variant_initializers_must_match_the_declared_field_types() {
    let fixture = Fixture::new(
        "enum Shape { Circle { radius: i32 } } \
         func main() { let shape = Shape::Circle { radius: true }; }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn an_enum_value_does_not_unify_with_another_nominal_type() {
    let fixture = Fixture::new(
        "enum Shape { Empty } struct Marker {} \
         func main() { let marker: Marker = Shape::Empty; }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn associated_functions_on_enums_return_the_enum_type() {
    let (fixture, hir, results) = check_clean(
        "enum Mode { On, Off } \
         impl Mode { func on() -> Mode { Mode::On } } \
         func main() -> Mode { Mode::on() }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    assert_eq!(
        ty_text_with(&fixture, &hir, expr_ty(&results, tail(&hir, body))),
        "Mode"
    );
}

#[test]
fn a_tuple_variant_has_the_enum_type() {
    let (fixture, hir, results) = check_clean(
        "enum Option { Some(i32, i32), None } \
         func main() -> Option { Option::Some(40, 2) }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    assert_eq!(
        ty_text_with(&fixture, &hir, expr_ty(&results, tail(&hir, body))),
        "Option"
    );
}

#[test]
fn tuple_variant_arguments_are_checked_in_declaration_order() {
    let fixture =
        Fixture::new("enum Pair { Both(i32, bool) } func main() { let p = Pair::Both(true, 1); }");
    let _ = fixture.check(64);
    fixture.expect("E3001", "mismatched types");
    assert!(
        fixture
            .codes()
            .iter()
            .filter(|code| code.as_str() == "E3001")
            .count()
            >= 2,
        "both arguments mismatch their declared fields"
    );
}

#[test]
fn a_unit_variant_called_with_parentheses_is_rejected() {
    let fixture = Fixture::new("enum Mode { On } func main() { let mode = Mode::On(); }");
    let _ = fixture.check(64);
    fixture.expect("E2039", "without parentheses");
}
