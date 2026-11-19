//! Borrow checking of opt-in `Copy` values.
//!
//! Copying an aggregate duplicates the origins of its fields: a reference keeps
//! its loan alive, while a raw pointer carries no loan once it is copied. The
//! move-path checks must also treat the copy as a read, not as a move.

use super::support::{check_clean, check_errors};

const POINT: &str = "struct Point { x: i32, y: i32 } impl Copy for Point {}";

#[test]
fn copying_an_adt_with_a_reference_keeps_its_loan() {
    let fixture = check_errors(
        "struct Holder { pointer: *i32 } \
         impl Copy for Holder {} \
         func main() -> i32 { \
           let mut value = 1; \
           let holder = Holder { pointer: &value }; \
           let copy = holder; \
           value = 2; \
           *copy.pointer + value \
         }",
    );
    fixture.expect("E5001", "`value`");
}

#[test]
fn copying_an_adt_with_an_exclusive_reference_keeps_its_loan() {
    let fixture = check_errors(
        "struct Holder { pointer: *mut i32 } \
         impl Copy for Holder {} \
         func main() -> i32 { \
           let mut value = 1; \
           let holder = Holder { pointer: &mut value }; \
           let copy = holder; \
           value = 2; \
           *copy.pointer + value \
         }",
    );
    fixture.expect("E5008", "exclusively borrowed");
}

#[test]
fn copying_an_adt_with_a_raw_pointer_does_not_keep_a_loan() {
    check_clean(
        "struct Holder { pointer: *raw mut i32 } \
         impl Copy for Holder {} \
         func main() -> i32 { \
           let mut value = 1; \
           let holder = Holder { pointer: &mut value as *raw mut i32 }; \
           let copy = holder; \
           value = 2; \
           let observed = unsafe { *copy.pointer }; \
           observed + value \
         }",
    );
}

#[test]
fn reading_a_field_of_a_copy_adt_is_a_copy_not_a_move() {
    check_clean(
        "struct Inner { value: i32 } \
         impl Copy for Inner {} \
         struct Pair { left: Inner, right: Inner } \
         func main() -> i32 { \
           let pair = Pair { \
             left: Inner { value: 20 }, \
             right: Inner { value: 22 }, \
           }; \
           let left = pair.left; \
           left.value + pair.right.value \
         }",
    );
}

#[test]
fn indexing_a_copy_array_copies_without_the_element_move_error() {
    let source = format!(
        "{POINT} \
         func main() -> i32 {{ \
           let points = [Point {{ x: 20, y: 1 }}, Point {{ x: 1, y: 20 }}]; \
           let first = points[0]; \
           first.x + points[1].y \
         }}"
    );
    check_clean(&source);
}

#[test]
fn reading_and_dropping_a_copy_static_is_allowed() {
    let source = format!(
        "{POINT} \
         static HOME: Point = Point {{ x: 40, y: 2 }}; \
         func main() -> i32 {{ \
           drop(HOME); \
           HOME.x + HOME.y \
         }}"
    );
    check_clean(&source);
}

#[test]
fn if_let_keeps_a_copy_scrutinee_usable() {
    check_clean(
        "enum Maybe { Some(Point), None } \
         impl Copy for Maybe {} \
         struct Point { x: i32, y: i32 } \
         impl Copy for Point {} \
         func main() -> i32 { \
           let maybe = Maybe::Some(Point { x: 40, y: 2 }); \
           let first = if let Maybe::Some(point) = maybe { point.x } else { 0 }; \
           let second = if let Maybe::Some(point) = maybe { point.y } else { 0 }; \
           first + second \
         }",
    );
}

#[test]
fn a_generic_match_guard_can_read_a_copy_binding() {
    check_clean(
        "struct Wrapper<T> { value: T } \
         impl<T: Copy> Copy for Wrapper<T> {} \
         func always<T>(value: T) -> bool { true } \
         func pick<T: Copy>(wrapper: Wrapper<T>) -> T { \
           match wrapper { \
             Wrapper { value } if always(value) => value, \
             Wrapper { value } => value, \
           } \
         }",
    );
}
