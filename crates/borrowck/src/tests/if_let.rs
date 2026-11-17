//! Borrow checking `if let` expressions.

use super::support::{check_clean, check_errors};

#[test]
fn an_if_let_binding_moves_the_payload_and_ends_the_scrutinee() {
    let fixture = check_errors(
        "struct Payload { value: i32 } \
         enum Option { None, Some(Payload) } \
         func consume(option: Option) -> i32 { 0 } \
         func main(option: Option) -> i32 { \
           if let Option::Some(payload) = option { let copied = payload.value; } \
           consume(option) \
         }",
    );
    fixture.expect("E5005", "moved");
}

#[test]
fn a_copy_field_can_be_bound_through_a_shared_reference() {
    let _ = check_clean(
        "enum Option { None, Some(i32) } \
         func read(option: *Option) -> i32 { \
           if let Option::Some(inner) = option { inner } else { 0 } \
         }",
    );
}
