//! Borrow checking of closure captures.

use super::support::{check_clean, check_errors};

#[test]
fn a_captured_place_cannot_be_written_while_the_closure_lives() {
    let fixture = check_errors(
        "func main() -> i32 { \
             let mut value = 10; \
             let read = || value; \
             value = 32; \
             read() \
         }",
    );
    fixture.expect("E5001", "cannot assign to `value` because it is borrowed");
}

#[test]
fn a_nested_closure_may_reborrow_a_shared_capture() {
    check_clean(
        "func main() -> i32 { \
             let value = 40; \
             let outer = || { let inner = || value + 2; inner() }; \
             outer() \
         }",
    );
}

#[test]
fn an_fn_mut_closure_may_write_its_capture() {
    check_clean(
        "func main() -> i32 { \
             let mut total = 0; \
             let mut add = |value: i32| { total += value; }; \
             add(2); \
             total \
         }",
    );
}
