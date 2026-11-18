//! Operator tests.

use super::support::{
    check_clean, check_errors, expect_ty, expr_statement, expr_ty, function, statements, tail,
};

#[test]
fn arithmetic_propagates_the_expected_type() {
    let (fixture, hir, results) = check_clean("func main() -> i64 { 1 + 2 }");
    let (_, body) = function(&fixture, &hir, "main");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "i64");
}

#[test]
fn mixed_width_arithmetic_is_rejected() {
    let (fixture, _, _) = check_errors("func main() -> i32 { let x = 1i32 + 1i64; 0 }");
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn arithmetic_on_bool_is_rejected() {
    let (fixture, _, _) = check_errors("func main() -> i32 { let x = true + false; 0 }");
    fixture.expect("E3002", "operator `+`");
}

#[test]
fn comparisons_produce_bool() {
    let (fixture, hir, results) = check_clean("func main() -> bool { 1 < 2 }");
    let (_, body) = function(&fixture, &hir, "main");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "bool");
}

#[test]
fn ordering_bool_is_rejected() {
    let (fixture, _, _) = check_errors("func main() -> bool { true < false }");
    fixture.expect("E3002", "operator `<`");
}

#[test]
fn logical_operators_require_bool() {
    let (fixture, _, _) = check_errors("func main() -> bool { 1 && true }");
    fixture.expect("E3002", "logical operator `&&`");
}

#[test]
fn bitwise_operators_require_integers() {
    let (fixture, _, _) = check_errors("func main() -> bool { true & false }");
    fixture.expect("E3002", "operator `&`");
}

#[test]
fn shifts_require_matching_integer_types() {
    let (fixture, _, _) = check_errors("func main() -> i32 { 1 << true }");
    fixture.expect("E3001", "mismatched types");

    let (fixture, hir, results) = check_clean("func main() -> u8 { 1u8 << 4u8 }");
    let (_, body) = function(&fixture, &hir, "main");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "u8");
}

#[test]
fn unary_negation_requires_a_signed_or_float_type() {
    let (fixture, hir, results) = check_clean("func main() -> f32 { -1.5 }");
    let (_, body) = function(&fixture, &hir, "main");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "f32");

    let (fixture, _, _) = check_errors("func main() -> u8 { -1u8 }");
    fixture.expect("E3002", "cannot negate");
}

#[test]
fn equality_accepts_aggregates_and_reference_contents() {
    check_clean(
        "struct Point { x: i32, y: i32 } \
         enum Shape { Dot, Line(i32) } \
         func main() -> bool { \
           let a: Point = Point { x: 1, y: 2 }; \
           let b: Point = Point { x: 1, y: 2 }; \
           let text: *str = \"seal\"; \
           let values: [i32; 2] = [1, 2]; \
           let slice: *[i32] = &values[..]; \
           let pair: (i32, bool) = (1, true); \
           let dot: Shape = Shape::Dot; \
           let line: Shape = Shape::Line(1); \
           a == b \
             && text == text \
             && slice == slice \
             && values == values \
             && pair == pair \
             && dot != line \
             && () == () \
             && 'a' == 'a' \
             && 1.5 != 2.5 \
         }",
    );
}

#[test]
fn equality_reads_pointees_and_statics() {
    check_clean(
        "struct Point { x: i32, y: i32 } \
         static ORIGIN: Point = Point { x: 0, y: 0 }; \
         func main() -> bool { \
           let a: Point = Point { x: 1, y: 2 }; \
           let b: Point = Point { x: 1, y: 2 }; \
           let left: *Point = &a; \
           let right: *Point = &b; \
           (*left == *right) && (a == ORIGIN) && (ORIGIN == b) \
         }",
    );
}

#[test]
fn equality_rejects_thin_references() {
    let (fixture, _, _) = check_errors(
        "func main() -> bool { \
           let a: i32 = 1; \
           let b: i32 = 2; \
           let left: *i32 = &a; \
           let right: *i32 = &b; \
           left == right \
         }",
    );
    fixture.expect("E3002", "operator `==` cannot be applied to type `*i32`");
}

#[test]
fn equality_rejects_a_field_without_equality() {
    let (fixture, _, _) = check_errors(
        "struct Point { x: i32 } \
         struct Holder { point: *Point } \
         func main() -> bool { \
           let a: Point = Point { x: 1 }; \
           let p: *Point = &a; \
           let h: Holder = Holder { point: p }; \
           h == h \
         }",
    );
    fixture.expect("E3002", "operator `==` cannot be applied to type `Holder`");

    let (fixture, _, _) = check_errors(
        "struct Holder { values: [*mut i32; 2] } \
         func main() -> bool { \
           let h: Holder = Holder { values: [] }; \
           h != h \
         }",
    );
    fixture.expect("E3002", "operator `!=` cannot be applied to type `Holder`");
}

#[test]
fn equality_rejects_trait_objects() {
    let (fixture, _, _) = check_errors(
        "trait Area { func area(self: *Self) -> i32; } \
         struct Square { side: i32 } \
         impl Area for Square { func area(self: *Square) -> i32 { (*self).side } } \
         func main() -> bool { \
           let a: Square = Square { side: 2 }; \
           let left: *dyn Area = &a; \
           left == left \
         }",
    );
    fixture.expect(
        "E3002",
        "operator `==` cannot be applied to type `*dyn Area`",
    );
}

#[test]
fn equality_rejects_type_parameters() {
    let (fixture, _, _) = check_errors("func same<T>(a: T, b: T) -> bool { a == b }");
    fixture.expect("E3041", "operator `==` is not available");

    let (fixture, _, _) = check_errors(
        "struct Pair<T> { left: T, right: T } \
         func same<T>(a: Pair<T>, b: Pair<T>) -> bool { a == b }",
    );
    fixture.expect("E3002", "operator `==` cannot be applied to type `Pair<T>`");
}

#[test]
fn logical_not_and_bitwise_not_have_their_own_families() {
    let (fixture, hir, results) = check_clean("func main() -> bool { !false }");
    let (_, body) = function(&fixture, &hir, "main");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "bool");

    let (fixture, hir, results) = check_clean("func main() -> u8 { ~0u8 }");
    let (_, body) = function(&fixture, &hir, "main");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "u8");

    let (fixture, _, _) = check_errors("func main() -> bool { !1 }");
    fixture.expect("E3002", "`!` requires a `bool`");
}

#[test]
fn compound_assignment_checks_the_operator_family() {
    let (fixture, _, _) = check_errors("func main() { let mut flag = true; flag &= false; }");
    fixture.expect("E3002", "compound assignment `&=`");

    let (fixture, hir, results) =
        check_clean("func main() -> i32 { let mut x = 1; x += 2; x <<= 1; x }");
    let (_, body) = function(&fixture, &hir, "main");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "i32");
}

#[test]
fn assignments_and_updates_have_unit_type() {
    let (fixture, hir, results) = check_clean("func main() { let mut x = 0; x = 1; }");
    let (_, body) = function(&fixture, &hir, "main");
    let assignment = expr_statement(&hir, statements(&hir, body)[1]);
    expect_ty(&hir, expr_ty(&results, assignment), "()");
}

#[test]
fn increment_requires_a_mutable_numeric_place() {
    let (fixture, _, _) = check_errors("func main() { let flag = true; flag++; }");
    fixture.expect("E3004", "not declared as mutable");
}
