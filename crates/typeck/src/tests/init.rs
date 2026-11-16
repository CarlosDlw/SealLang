//! Definite-initialization tests.

use super::support::{
    check_clean, check_errors, expect_ty, expr_ty, function, let_binding, local_ty, statements,
    tail,
};

#[test]
fn assignment_initializes_a_binding() {
    let (fixture, hir, results) = check_clean("func main() -> i32 { let x: i32; x = 1; x }");
    let (_, body) = function(&fixture, &hir, "main");
    let binding = let_binding(&hir, statements(&hir, body)[0]);
    expect_ty(&hir, local_ty(&results, binding), "i32");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "i32");
}

#[test]
fn reading_an_uninitialized_binding_is_reported() {
    let (fixture, _, _) = check_errors("func main() -> i32 { let x: i32; x }");
    fixture.expect("E3011", "possibly uninitialized");
}

#[test]
fn both_if_branches_initialize_a_binding() {
    let (fixture, _, _) = check_clean(
        "func main(flag: bool) -> i32 { let x: i32; \
         if flag { x = 1; } else { x = 2; } x }",
    );
    let _ = fixture;
}

#[test]
fn one_if_branch_does_not_initialize_a_binding() {
    let (fixture, _, _) =
        check_errors("func main(flag: bool) -> i32 { let x: i32; if flag { x = 1; } x }");
    fixture.expect("E3011", "possibly uninitialized");
}

#[test]
fn a_diverging_branch_does_not_have_to_initialize() {
    let (fixture, _, _) = check_clean(
        "func main(flag: bool) -> i32 { let x: i32; \
         if flag { return 0; } else { x = 1; } x }",
    );
    let _ = fixture;
}

#[test]
fn assignments_inside_a_while_do_not_count_after_the_loop() {
    let (fixture, _, _) =
        check_errors("func main() -> i32 { let x: i32; while false { x = 1; } x }");
    fixture.expect("E3011", "possibly uninitialized");
}

#[test]
fn an_inner_shadow_does_not_initialize_the_outer_binding() {
    let (fixture, _, _) =
        check_errors("func main() -> i32 { let x: i32; if true { let x = 1; } x }");
    fixture.expect("E3011", "possibly uninitialized");
}

#[test]
fn parameters_are_initialized() {
    let (fixture, _, _) = check_clean("func identity(value: i32) -> i32 { value }");
    let _ = fixture;
}

#[test]
fn immutable_bindings_cannot_be_assigned() {
    let (fixture, _, _) = check_errors("func main() { let value = 1; value = 2; }");
    fixture.expect("E3004", "not declared as mutable");
}

#[test]
fn parameters_are_immutable() {
    let (fixture, _, _) = check_errors("func f(value: i32) { value = 1; }");
    fixture.expect("E3004", "not declared as mutable");
}

#[test]
fn functions_are_not_assignment_targets() {
    let (fixture, _, _) = check_errors("func f() {} func main() { f = 1; }");
    fixture.expect("E3005", "cannot assign to a function");
}

#[test]
fn compound_assignment_and_update_read_the_target() {
    let (fixture, _, _) = check_errors("func main() -> i32 { let x: i32; x += 1; x }");
    fixture.expect("E3011", "possibly uninitialized");

    let (fixture, _, _) = check_errors("func main() -> i32 { let x: i32; x++; x }");
    fixture.expect("E3011", "possibly uninitialized");
}

#[test]
fn reads_inside_conditions_are_checked() {
    let (fixture, _, _) = check_errors("func main() { let x: i32; if x > 0 { } }");
    fixture.expect("E3011", "possibly uninitialized");
}
