//! Tuple field precision, partial moves, and references inside tuples.

use super::support::{check_clean, check_errors};

#[test]
fn borrowing_a_tuple_field_leaves_siblings_writable() {
    check_clean(
        "func f() -> i32 {\n\
           let mut pair = (1, 2);\n\
           let r = &pair.0;\n\
           pair.1 = 3;\n\
           *r\n\
         }\n",
    );
}

#[test]
fn borrowing_the_whole_tuple_blocks_writing_a_field() {
    let fixture = check_errors(
        "func f() -> i32 {\n\
           let mut pair = (1, 2);\n\
           let r = &pair;\n\
           pair.0 = 3;\n\
           (*r).0\n\
         }\n",
    );
    fixture.expect("E5001", "`pair.0`");
}

#[test]
fn moving_a_sibling_tuple_field_while_one_is_borrowed_is_allowed() {
    check_clean(
        "struct Point { x: i32 }\n\
         func f() -> i32 {\n\
           let pair = (Point { x: 1 }, Point { x: 2 });\n\
           let b = &pair.1;\n\
           let a = pair.0;\n\
           (*b).x + a.x\n\
         }\n",
    );
}

#[test]
fn moving_a_borrowed_tuple_field_is_rejected() {
    let fixture = check_errors(
        "struct Point { x: i32 }\n\
         func f() -> i32 {\n\
           let pair = (Point { x: 1 }, Point { x: 2 });\n\
           let r = &pair.1;\n\
           let b = pair.1;\n\
           (*r).x + b.x\n\
         }\n",
    );
    fixture.expect("E5002", "`pair.1`");
}

#[test]
fn moving_the_whole_tuple_after_a_field_move_is_rejected() {
    let fixture = check_errors(
        "struct Point { x: i32 }\n\
         func take(pair: (Point, Point)) -> i32 { pair.0.x + pair.1.x }\n\
         func f() -> i32 {\n\
           let pair = (Point { x: 1 }, Point { x: 2 });\n\
           let a = pair.0;\n\
           take(pair) + a.x\n\
         }\n",
    );
    fixture.expect("E5005", "`pair`");
}

#[test]
fn a_tuple_carries_the_loans_of_its_reference_elements() {
    let fixture = check_errors(
        "func f() -> i32 {\n\
           let mut x = 1;\n\
           let pair = (&x, 2);\n\
           let copy = pair;\n\
           x = 2;\n\
           *copy.0\n\
         }\n",
    );
    fixture.expect("E5001", "`x`");
}

#[test]
fn overwriting_a_reference_element_ends_the_old_loan() {
    check_clean(
        "func f() -> i32 {\n\
           let mut x = 1;\n\
           let y = 2;\n\
           let mut pair = (&x, 0);\n\
           pair.0 = &y;\n\
           x = 3;\n\
           *pair.0\n\
         }\n",
    );
}

#[test]
fn returning_a_tuple_that_borrows_a_local_is_rejected() {
    let fixture = check_errors(
        "func f() -> (*i32, i32) {\n\
           let x = 1;\n\
           (&x, 0)\n\
         }\n",
    );
    fixture.expect("E5004", "`x`");
}
