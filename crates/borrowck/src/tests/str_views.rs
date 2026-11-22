//! Loans created by string and byte-slice views.

use super::support::{check_clean, check_errors};

#[test]
fn overlapping_shared_views_are_allowed() {
    check_clean(
        "func f(text: *str) -> i32 { \
         let first = text.slice(0, 2); \
         let second = text.slice(1, 3); \
         if first == second { 1 } else { 0 } }",
    );
}

#[test]
fn length_and_boundary_reads_do_not_keep_a_view_alive() {
    check_clean(
        "func f(text: *str) -> i32 { \
         let view = text.slice(0, 1); \
         let length = text.len(); \
         if text.is_char_boundary(length) && view.len() == 1 { 1 } else { 0 } }",
    );
}

#[test]
fn scalar_reads_do_not_keep_a_view_alive() {
    check_clean(
        "func f(text: *str) -> bool { \
         let view = text.slice(0, 1); \
         let first = text.char_at(0); \
         view.len() == 1 && first == 'h' }",
    );
}

#[test]
fn returning_a_view_of_a_local_byte_array_is_rejected() {
    check_errors(
        "func f() -> *str { \
         let data: [u8; 2] = [104u8, 105u8]; \
         let bytes: *[u8] = &data[..]; \
         unsafe { bytes.as_str_unchecked() } }",
    )
    .expect("E5004", "local");
}

#[test]
fn returning_a_view_of_a_string_literal_is_allowed() {
    check_clean("func f() -> *str { let text: *str = \"hello\"; text.slice(0, 2) }");
}

#[test]
fn returning_a_view_of_a_static_string_is_allowed() {
    check_clean(
        "static TEXT: *str = \"hello\"; \
         func f() -> *str { TEXT.slice(0, 2) }",
    );
}

#[test]
fn writing_the_source_while_a_byte_view_is_live_is_rejected() {
    let fixture = check_errors(
        "func f() -> i32 { \
         let mut data: [u8; 2] = [104u8, 105u8]; \
         let bytes: *[u8] = &data[..]; \
         let view = unsafe { bytes.as_str_unchecked() }; \
         data[0] = 0; \
         if view.len() == 2 { 1 } else { 0 } }",
    );
    fixture.expect("E5001", "borrowed");
}
