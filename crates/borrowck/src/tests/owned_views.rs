//! Loans attached to the owner of an `owned_view`.

use super::support::{Fixture, check_clean, check_errors};

const BUFFER: &str = "struct Buffer { pointer: *raw i32 }\n";
const MUT_BUFFER: &str = "struct Buffer { pointer: *raw mut i32 }\n";

#[test]
fn a_view_returned_through_a_borrowed_owner_is_allowed() {
    check_clean(&format!(
        "{BUFFER}\
         func view(buffer: *Buffer) -> *i32 {{\
           unsafe {{ owned_view(*buffer, (*buffer).pointer) }}\
         }}\
         func get(buffer: *Buffer) -> *i32 {{ view(buffer) }}"
    ));
}

#[test]
fn a_view_of_a_local_owner_cannot_escape() {
    let fixture = check_errors(&format!(
        "{BUFFER}\
         func leak() -> *i32 {{\
           let value = 1;\
           let buffer = Buffer {{ pointer: &value as *raw i32 }};\
           unsafe {{ owned_view(buffer, buffer.pointer) }}\
         }}"
    ));
    fixture.expect("E5004", "local");
}

#[test]
fn moving_the_owner_conflicts_with_a_live_view() {
    let fixture = Fixture::new(&format!(
        "{BUFFER}\
         func f() -> i32 {{\
           let value = 1;\
           let buffer = Buffer {{ pointer: &value as *raw i32 }};\
           let view = unsafe {{ owned_view(buffer, buffer.pointer) }};\
           drop(buffer);\
           *view\
         }}"
    ));
    fixture.check_with_destructors(&["Buffer"]);
    fixture.expect("E5002", "because it is borrowed");
}

#[test]
fn writing_the_owner_conflicts_with_a_shared_view() {
    let fixture = check_errors(&format!(
        "{BUFFER}\
         func f() -> i32 {{\
           let value = 1;\
           let mut buffer = Buffer {{ pointer: &value as *raw i32 }};\
           let view = unsafe {{ owned_view(buffer, buffer.pointer) }};\
           buffer.pointer = 0usize as *raw i32;\
           *view\
         }}"
    ));
    fixture.expect("E5001", "because it is borrowed");
}

#[test]
fn reading_the_owner_conflicts_with_an_exclusive_view() {
    let fixture = check_errors(&format!(
        "{MUT_BUFFER}\
         func f() -> i32 {{\
           let mut value = 1;\
           let mut buffer = Buffer {{ pointer: &mut value as *raw mut i32 }};\
           let view = unsafe {{ owned_view(buffer, buffer.pointer) }};\
           let pointer = buffer.pointer;\
           let _ = pointer;\
           *view\
         }}"
    ));
    fixture.expect("E5007", "is exclusively borrowed");
}

#[test]
fn shared_views_can_overlap_and_exclusive_views_cannot() {
    check_clean(&format!(
        "{BUFFER}\
         func f() -> i32 {{\
           let value = 1;\
           let buffer = Buffer {{ pointer: &value as *raw i32 }};\
           let a = unsafe {{ owned_view(buffer, buffer.pointer) }};\
           let b = unsafe {{ owned_view(buffer, buffer.pointer) }};\
           *a + *b\
         }}"
    ));

    let fixture = check_errors(&format!(
        "{MUT_BUFFER}\
         func f() -> i32 {{\
           let mut value = 1;\
           let mut buffer = Buffer {{ pointer: &mut value as *raw mut i32 }};\
           let shared = unsafe {{ owned_view(buffer, buffer.pointer as *raw i32) }};\
           let exclusive = unsafe {{ owned_view(buffer, buffer.pointer) }};\
           *shared + *exclusive\
         }}"
    ));
    fixture.expect("E5008", "already borrowed");
}

#[test]
fn a_view_can_be_reborrowed() {
    check_clean(&format!(
        "{BUFFER}\
         func f() -> i32 {{\
           let value = 1;\
           let buffer = Buffer {{ pointer: &value as *raw i32 }};\
           let view = unsafe {{ owned_view(buffer, buffer.pointer) }};\
           let reborrow = &*view;\
           *reborrow\
         }}"
    ));
}

#[test]
fn the_owner_can_drop_after_the_views_last_use() {
    check_clean(&format!(
        "{BUFFER}\
         func f() -> i32 {{\
           let value = 1;\
           let buffer = Buffer {{ pointer: &value as *raw i32 }};\
           let view = unsafe {{ owned_view(buffer, buffer.pointer) }};\
           let result = *view;\
           drop(buffer);\
           result\
         }}"
    ));
}
