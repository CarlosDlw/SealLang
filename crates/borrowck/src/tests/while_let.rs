//! Borrow checking `while let` statements.

use super::support::{check_clean, check_errors};

#[test]
fn a_fresh_scrutinee_can_be_bound_on_every_iteration() {
    let _ = check_clean(
        "enum Option { None, Some(i32) } \
         func next() -> Option { Option::Some(1) } \
         func main() -> i32 { \
           let mut total = 0; \
           while let Option::Some(value) = next() { \
             total += value; \
             if total > 3 { break; } \
           } \
           total \
         }",
    );
}

#[test]
fn moving_a_loop_carried_scrutinee_is_rejected() {
    let fixture = check_errors(
        "enum Option { None, Some(i32) } \
         func main(option: Option) -> i32 { \
           let mut total = 0; \
           while let Option::Some(value) = option { total += value; } \
           total \
         }",
    );
    fixture.expect("E5005", "moved");
}
