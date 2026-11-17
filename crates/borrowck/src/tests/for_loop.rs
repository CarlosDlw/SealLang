//! Borrow checking `for` loops.

use super::support::{check_clean, check_errors};

#[test]
fn mutating_a_sequence_while_iterating_a_shared_loan_is_rejected() {
    let fixture = check_errors(
        "func main() -> i32 { let mut values = [1, 2]; for value in &values { values[0] = 9; } 0 }",
    );
    fixture.expect("E5001", "borrowed");
}

#[test]
fn consuming_a_non_copy_array_by_value_is_accepted() {
    let _ = check_clean(
        "struct Point { x: i32 } \
         func main() -> i32 { \
           let points = [Point { x: 40 }, Point { x: 2 }]; \
           let mut total = 0; \
           for point in points { total += point.x; } \
           total \
         }",
    );
}

#[test]
fn moving_a_non_copy_array_into_a_loop_ends_its_use() {
    let fixture = check_errors(
        "struct Point { x: i32 } \
         func main(points: [Point; 1]) -> i32 { for point in points { } points[0].x }",
    );
    fixture.expect("E5005", "moved");
}

#[test]
fn exclusive_iteration_can_write_through_the_element_reference() {
    let _ = check_clean(
        "func main(values: *mut [i32]) -> i32 { \
           let mut total = 0; \
           for value in values { *value += 1; total += *value; } \
           total \
         }",
    );
}
