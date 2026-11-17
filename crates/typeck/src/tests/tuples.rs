//! Tuple inference, positional access, patterns, and diagnostics.

use crate::PlaceProjection;

use super::support::{
    Fixture, check_clean, expr_ty, function, let_binding, statements, tail, ty_text_with,
};

#[test]
fn a_tuple_literal_has_a_structural_type_in_source_order() {
    let (fixture, hir, results) = check_clean("func main() -> (i32, bool) { (1, true) }");
    let (_, body) = function(&fixture, &hir, "main");
    let expression = tail(&hir, body);
    assert_eq!(
        ty_text_with(&fixture, &hir, expr_ty(&results, expression)),
        "(i32, bool)"
    );
}

#[test]
fn a_tuple_field_access_is_a_place_with_the_element_type() {
    let (fixture, hir, results) =
        check_clean("func main() -> i32 { let pair = (1, true); pair.0 }");
    let (_, body) = function(&fixture, &hir, "main");
    let expression = tail(&hir, body);
    assert_eq!(
        ty_text_with(&fixture, &hir, expr_ty(&results, expression)),
        "i32"
    );
    let place = results.place(expression).expect("tuple field is a place");
    assert!(matches!(
        place.projections.as_slice(),
        [PlaceProjection::TupleField(0)]
    ));
}

#[test]
fn nested_field_access_keeps_each_projection() {
    let (fixture, hir, results) =
        check_clean("func main() -> i32 { let nested = ((1, 2), 3); nested.0.1 }");
    let (_, body) = function(&fixture, &hir, "main");
    let expression = tail(&hir, body);
    assert_eq!(
        ty_text_with(&fixture, &hir, expr_ty(&results, expression)),
        "i32"
    );
    let place = results.place(expression).expect("nested access is a place");
    assert!(matches!(
        place.projections.as_slice(),
        [
            PlaceProjection::TupleField(0),
            PlaceProjection::TupleField(1)
        ]
    ));
}

#[test]
fn an_annotation_guides_inference_for_each_element() {
    let (fixture, hir, results) =
        check_clean("func main() -> i32 { let pair: (i32, bool) = (1, true); pair.0 }");
    let (_, body) = function(&fixture, &hir, "main");
    let expression = tail(&hir, body);
    assert_eq!(
        ty_text_with(&fixture, &hir, expr_ty(&results, expression)),
        "i32"
    );
}

#[test]
fn a_tuple_index_outside_the_arity_reports_e3036() {
    let fixture = Fixture::new("func main() -> i32 { let pair = (1, true); pair.2 }");
    let _ = fixture.check(64);
    fixture.expect("E3036", "out of bounds");
}

#[test]
fn positional_access_on_a_non_tuple_reports_e3015() {
    let fixture = Fixture::new("func main() -> i32 { let value = 1; value.0 }");
    let _ = fixture.check(64);
    fixture.expect("E3015", "has no tuple elements");

    let fixture = Fixture::new("func main() -> i32 { let value = (); value.0 }");
    let _ = fixture.check(64);
    fixture.expect("E3036", "unit value");
}

#[test]
fn a_reference_needs_an_explicit_deref_before_a_tuple_field() {
    let fixture = Fixture::new("func main(pair: *(i32, i32)) -> i32 { pair.0 }");
    let _ = fixture.check(64);
    fixture.expect("E3015", "through a reference");

    let (fixture, hir, results) = check_clean("func main(pair: *(i32, i32)) -> i32 { (*pair).0 }");
    let (_, body) = function(&fixture, &hir, "main");
    let expression = tail(&hir, body);
    assert_eq!(
        ty_text_with(&fixture, &hir, expr_ty(&results, expression)),
        "i32"
    );
}

#[test]
fn a_tuple_pattern_requires_the_declared_arity() {
    let fixture =
        Fixture::new("func main(pair: (i32, i32)) -> i32 { match pair { (a, b, c) => a } }");
    let _ = fixture.check(64);
    fixture.expect("E3037", "3 element(s)");
}

#[test]
fn a_tuple_pattern_on_a_non_tuple_reports_e3037() {
    let fixture = Fixture::new("func main(value: i32) -> i32 { match value { (a, b) => a } }");
    let _ = fixture.check(64);
    fixture.expect("E3037", "needs a tuple");
}

#[test]
fn tuple_patterns_through_references_follow_the_copy_rule() {
    check_clean(
        "func main(pair: *(i32, i32)) -> i32 { \
           match pair { (left, right) => left + right } \
         }",
    );

    let fixture = Fixture::new(
        "struct Point { x: i32 } \
         func main(pair: *(Point, i32)) -> i32 { \
           match pair { (point, tag) => point.x + tag } \
         }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3018", "out of a reference");
}

#[test]
fn tuple_patterns_cover_every_element_and_are_exhaustive() {
    check_clean(
        "func main(pair: (i32, i32)) -> i32 { \
           match pair { (0, _) => 1, (_, 0) => 2, (_, _) => 3 } \
         }",
    );
}

#[test]
fn tuple_coverage_reports_the_missing_element_combination() {
    let fixture = Fixture::new(
        "enum Mode { On, Off }\n\
         func main(pair: (Mode, Mode)) -> i32 {\n\
           match pair {\n\
             (Mode::On, _) => 1,\n\
             (_, Mode::On) => 2,\n\
           }\n\
         }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3032", "(Mode::Off, Mode::Off)");
}

#[test]
fn tuple_copy_and_sized_queries_agree_with_every_element() {
    let (fixture, hir, results) = check_clean(
        "struct Point { x: i32 } \
         func main() -> i32 { let pair = (Point { x: 1 }, 2); pair.1 }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    let local = let_binding(&hir, statements(&hir, body)[0]);
    let local_ty = results.local_ty(local).expect("binding type");
    assert!(!hir.ty_ctxt().is_copy(local_ty));
    assert!(hir.ty_ctxt().is_sized(local_ty));

    let (fixture, hir, results) =
        check_clean("func main() -> i32 { let pair = (1, true); pair.0 }");
    let (_, body) = function(&fixture, &hir, "main");
    let local = let_binding(&hir, statements(&hir, body)[0]);
    let local_ty = results.local_ty(local).expect("binding type");
    assert!(hir.ty_ctxt().is_copy(local_ty));
    assert_eq!(ty_text_with(&fixture, &hir, local_ty), "(i32, bool)");
}

#[test]
fn tuple_elements_initialize_and_track_independently() {
    check_clean(
        "func main() -> i32 {\n\
           let mut pair: (i32, i32);\n\
           pair.0 = 4;\n\
           pair.1 = 5;\n\
           pair.0 + pair.1\n\
         }",
    );

    let fixture = Fixture::new(
        "func main() -> i32 {\n\
           let mut pair: (i32, i32);\n\
           pair.1 = 5;\n\
           pair.0\n\
         }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3011", "pair.0");
}

#[test]
fn the_unit_tuple_is_the_unit_type() {
    let (fixture, hir, results) = check_clean("func main() -> () { () }");
    let (_, body) = function(&fixture, &hir, "main");
    let expression = tail(&hir, body);
    let ty = expr_ty(&results, expression);
    assert!(matches!(hir.ty_ctxt().kind(ty), seal_ty::TyKind::Unit));
    assert_eq!(ty_text_with(&fixture, &hir, ty), "()");
}
