//! Move paths, field borrows, partial moves, and references inside aggregates.

use super::support::{check_clean, check_errors};

#[test]
fn borrowing_a_field_leaves_siblings_writable() {
    check_clean(
        "struct Point { x: i32, y: i32 }\n\
         func f() -> i32 {\n\
           let mut p = Point { x: 1, y: 2 };\n\
           let r = &p.x;\n\
           p.y = 3;\n\
           *r\n\
         }\n",
    );
}

#[test]
fn borrowing_the_whole_blocks_writing_a_field() {
    let fixture = check_errors(
        "struct Point { x: i32, y: i32 }\n\
         func f() -> i32 {\n\
           let mut p = Point { x: 1, y: 2 };\n\
           let r = &p;\n\
           p.x = 3;\n\
           (*r).x\n\
         }\n",
    );
    fixture.expect("E5001", "`p.x`");
}

#[test]
fn moving_a_sibling_field_while_one_field_is_borrowed_is_allowed() {
    check_clean(
        "struct Inner { v: i32 }\n\
         struct Pair { a: Inner, b: Inner }\n\
         func f() -> i32 {\n\
           let p = Pair { a: Inner { v: 1 }, b: Inner { v: 2 } };\n\
           let b = &p.b;\n\
           let a = p.a;\n\
           (*b).v + a.v\n\
         }\n",
    );
}

#[test]
fn moving_a_borrowed_field_is_rejected() {
    let fixture = check_errors(
        "struct Inner { v: i32 }\n\
         struct Pair { a: Inner }\n\
         func f() -> i32 {\n\
           let p = Pair { a: Inner { v: 1 } };\n\
           let r = &p.a.v;\n\
           let a = p.a;\n\
           *r + a.v\n\
         }\n",
    );
    fixture.expect("E5002", "`p.a`");
}

#[test]
fn moving_the_whole_after_a_field_move_is_rejected() {
    let fixture = check_errors(
        "struct Inner { v: i32 }\n\
         struct Pair { a: Inner, b: Inner }\n\
         func take(p: Pair) -> i32 { p.a.v + p.b.v }\n\
         func f() -> i32 {\n\
           let p = Pair { a: Inner { v: 1 }, b: Inner { v: 2 } };\n\
           let a = p.a;\n\
           take(p) + a.v\n\
         }\n",
    );
    fixture.expect("E5005", "`p`");
}

#[test]
fn borrowing_the_whole_after_a_field_move_is_rejected() {
    let fixture = check_errors(
        "struct Inner { v: i32 }\n\
         struct Pair { a: Inner, b: Inner }\n\
         func f() -> i32 {\n\
           let p = Pair { a: Inner { v: 1 }, b: Inner { v: 2 } };\n\
           let a = p.a;\n\
           let r = &p;\n\
           (*r).b.v + a.v\n\
         }\n",
    );
    fixture.expect("E5006", "`p`");
}

#[test]
fn reading_an_unmoved_field_after_a_sibling_move_is_allowed() {
    check_clean(
        "struct Inner { v: i32 }\n\
         struct Pair { a: Inner, b: Inner }\n\
         func f() -> i32 {\n\
           let p = Pair { a: Inner { v: 1 }, b: Inner { v: 2 } };\n\
           let a = p.a;\n\
           p.b.v + a.v\n\
         }\n",
    );
}

#[test]
fn returning_an_aggregate_that_borrows_a_local_is_rejected() {
    let fixture = check_errors(
        "struct Ref { value: *i32 }\n\
         func f() -> Ref {\n\
           let x = 1;\n\
           Ref { value: &x }\n\
         }\n",
    );
    fixture.expect("E5004", "`x`");
}

#[test]
fn moving_an_aggregate_carries_its_inner_loans() {
    let fixture = check_errors(
        "struct Ref { value: *i32 }\n\
         func f() -> i32 {\n\
           let mut x = 1;\n\
           let r = Ref { value: &x };\n\
           let s = r;\n\
           x = 2;\n\
           *s.value\n\
         }\n",
    );
    fixture.expect("E5001", "`x`");
}

#[test]
fn overwriting_a_reference_field_ends_the_old_loan() {
    check_clean(
        "struct Ref { value: *i32 }\n\
         func f() -> i32 {\n\
           let mut x = 1;\n\
           let y = 2;\n\
           let mut r = Ref { value: &x };\n\
           r.value = &y;\n\
           x = 3;\n\
           *r.value\n\
         }\n",
    );
}

#[test]
fn a_borrowed_field_can_be_reassigned_after_its_last_use() {
    check_clean(
        "struct Point { x: i32, y: i32 }\n\
         func f() -> i32 {\n\
           let mut p = Point { x: 1, y: 2 };\n\
           let r = &p.x;\n\
           let read = *r;\n\
           p.x = 4;\n\
           p.x + p.y + read\n\
         }\n",
    );
}

#[test]
fn an_aggregate_passed_by_value_keeps_inner_loans_alive() {
    let fixture = check_errors(
        "struct Ref { value: *i32 }\n\
         func identity(r: Ref) -> Ref { r }\n\
         func f() -> i32 {\n\
           let mut x = 1;\n\
           let r = Ref { value: &x };\n\
           let s = identity(r);\n\
           x = 2;\n\
           *s.value\n\
         }\n",
    );
    fixture.expect("E5001", "`x`");
}
