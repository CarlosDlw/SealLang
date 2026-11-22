//! Phase 2 matrix for owned-view loans: escapes, aggregates, branches,
//! closures, field owners, slice/string owners, and static owners.

use super::support::{Fixture, check_clean, check_errors};

const BUFFER: &str = "struct Buffer { pointer: *raw i32 }\n";
const SLICE_BUFFER: &str = "struct SliceBuffer { pointer: *raw [i32] }\n";
const MUT_SLICE_BUFFER: &str = "struct SliceBuffer { pointer: *raw mut [i32] }\n";
const TEXT_BUFFER: &str = "struct TextBuffer { pointer: *raw str }\n";
const MUT_TEXT_BUFFER: &str = "struct TextBuffer { pointer: *raw mut str }\n";

#[test]
fn a_view_passed_to_a_function_stays_usable() {
    check_clean(&format!(
        "{BUFFER}\
         func read(view: *i32) -> i32 {{ *view }}\
         func f() -> i32 {{\
           let value = 1;\
           let buffer = Buffer {{ pointer: &value as *raw i32 }};\
           let view = unsafe {{ owned_view(buffer, buffer.pointer) }};\
           let first = read(view);\
           let second = *view;\
           first + second\
         }}"
    ));
}

#[test]
fn a_view_copied_into_a_tuple_and_a_struct_keeps_the_owner_loan() {
    check_clean(&format!(
        "{BUFFER}\
         struct Holder {{ view: *i32 }}\
         func f() -> i32 {{\
           let value = 1;\
           let buffer = Buffer {{ pointer: &value as *raw i32 }};\
           let view = unsafe {{ owned_view(buffer, buffer.pointer) }};\
           let pair = (view, 2);\
           let moved_pair = pair;\
           let holder = Holder {{ view: view }};\
           let moved_holder = holder;\
           *moved_pair.0 + *moved_holder.view\
         }}"
    ));
}

#[test]
fn moving_the_whole_owner_conflicts_with_a_view_of_its_field() {
    let fixture = check_errors(&format!(
        "{BUFFER}\
         struct Outer {{ buffer: Buffer, other: i32 }}\
         func f() -> i32 {{\
           let value = 1;\
           let outer = Outer {{\
             buffer: Buffer {{ pointer: &value as *raw i32 }},\
             other: 2,\
           }};\
           let view = unsafe {{ owned_view(outer.buffer, outer.buffer.pointer) }};\
           let moved = outer;\
           *view + moved.other\
         }}"
    ));
    fixture.expect("E5002", "because it is borrowed");
}

#[test]
fn moving_the_owner_field_conflicts_with_a_view_of_that_field() {
    let fixture = check_errors(&format!(
        "{BUFFER}\
         struct Outer {{ buffer: Buffer, other: i32 }}\
         func f() -> i32 {{\
           let value = 1;\
           let outer = Outer {{\
             buffer: Buffer {{ pointer: &value as *raw i32 }},\
             other: 2,\
           }};\
           let view = unsafe {{ owned_view(outer.buffer, outer.buffer.pointer) }};\
           let moved = outer.buffer;\
           let _ = moved;\
           *view\
         }}"
    ));
    fixture.expect("E5002", "because it is borrowed");
}

#[test]
fn reading_a_sibling_field_while_the_owner_field_is_viewed_is_allowed() {
    check_clean(&format!(
        "{BUFFER}\
         struct Outer {{ buffer: Buffer, other: i32 }}\
         func f() -> i32 {{\
           let value = 1;\
           let outer = Outer {{\
             buffer: Buffer {{ pointer: &value as *raw i32 }},\
             other: 2,\
           }};\
           let view = unsafe {{ owned_view(outer.buffer, outer.buffer.pointer) }};\
           *view + outer.other\
         }}"
    ));
}

#[test]
fn a_view_created_in_branches_keeps_the_owner_loan() {
    let fixture = check_errors(&format!(
        "{BUFFER}\
         func f(condition: bool) -> i32 {{\
           let value = 1;\
           let mut buffer = Buffer {{ pointer: &value as *raw i32 }};\
           let view = if condition {{\
             unsafe {{ owned_view(buffer, buffer.pointer) }}\
           }} else {{\
             unsafe {{ owned_view(buffer, buffer.pointer) }}\
           }};\
           buffer.pointer = 0usize as *raw i32;\
           *view\
         }}"
    ));
    fixture.expect("E5001", "because it is borrowed");
}

#[test]
fn a_closure_capturing_a_view_keeps_the_owner_loan() {
    check_clean(&format!(
        "{BUFFER}\
         func f() -> i32 {{\
           let value = 1;\
           let buffer = Buffer {{ pointer: &value as *raw i32 }};\
           let view = unsafe {{ owned_view(buffer, buffer.pointer) }};\
           let read = move || *view;\
           read()\
         }}"
    ));
}

#[test]
fn a_closure_capturing_a_view_conflicts_with_moving_the_owner() {
    let fixture = Fixture::new(&format!(
        "{BUFFER}\n         func f() -> i32 {{\
           let value = 1;\
           let buffer = Buffer {{ pointer: &value as *raw i32 }};\
           let view = unsafe {{ owned_view(buffer, buffer.pointer) }};\
           let read = move || *view;\
           drop(buffer);\
           read()\
         }}"
    ));
    fixture.check_with_destructors(&["Buffer"]);
    fixture.expect("E5002", "because it is borrowed");
}

#[test]
fn a_slice_view_borrows_its_owner() {
    let fixture = check_errors(&format!(
        "{SLICE_BUFFER}\
         func f() -> usize {{\
           let values = [1, 2];\
           let other = [3];\
           let mut buffer = SliceBuffer {{ pointer: &values[..] as *raw [i32] }};\
           let view = unsafe {{ owned_view(buffer, buffer.pointer) }};\
           buffer.pointer = &other[..] as *raw [i32];\
           len(view)\
         }}"
    ));
    fixture.expect("E5001", "because it is borrowed");
}

#[test]
fn shared_and_exclusive_slice_views_conflict() {
    let fixture = check_errors(&format!(
        "{MUT_SLICE_BUFFER}\
         func f() -> i32 {{\
           let mut values = [1, 2];\
           let mut buffer = SliceBuffer {{\
             pointer: &mut values[..] as *raw mut [i32],\
           }};\
           let shared = unsafe {{ owned_view(buffer, buffer.pointer as *raw [i32]) }};\
           let exclusive = unsafe {{ owned_view(buffer, buffer.pointer) }};\
           shared[0] + exclusive[0]\
         }}"
    ));
    fixture.expect("E5008", "already borrowed");
}

#[test]
fn moving_a_slice_owner_conflicts_with_a_live_view() {
    let fixture = Fixture::new(&format!(
        "{SLICE_BUFFER}\
         func f() -> usize {{\
           let values = [1, 2];\
           let buffer = SliceBuffer {{ pointer: &values[..] as *raw [i32] }};\
           let view = unsafe {{ owned_view(buffer, buffer.pointer) }};\
           drop(buffer);\
           len(view)\
         }}"
    ));
    fixture.check_with_destructors(&["SliceBuffer"]);
    fixture.expect("E5002", "because it is borrowed");
}

#[test]
fn shared_and_exclusive_string_views_conflict() {
    let fixture = check_errors(&format!(
        "{MUT_TEXT_BUFFER}\n         func f(buffer: *mut TextBuffer) -> bool {{\
           let shared = unsafe {{\
             owned_view(*buffer, (*buffer).pointer as *raw str)\
           }};\
           let exclusive = unsafe {{ owned_view(*buffer, (*buffer).pointer) }};\
           let _ = exclusive;\
           shared == \"hello\"\
         }}"
    ));
    fixture.expect("E5008", "already borrowed");
}

#[test]
fn a_string_view_borrows_its_owner() {
    let fixture = check_errors(&format!(
        "{TEXT_BUFFER}\
         func f() -> bool {{\
           let text = \"hello\";\
           let mut buffer = TextBuffer {{ pointer: text as *raw str }};\
           let view = unsafe {{ owned_view(buffer, buffer.pointer) }};\
           buffer.pointer = text as *raw str;\
           view == \"hello\"\
         }}"
    ));
    fixture.expect("E5001", "because it is borrowed");
}

#[test]
fn moving_a_string_owner_conflicts_with_a_live_view() {
    let fixture = Fixture::new(&format!(
        "{TEXT_BUFFER}\
         func f() -> bool {{\
           let text = \"hello\";\
           let buffer = TextBuffer {{ pointer: text as *raw str }};\
           let view = unsafe {{ owned_view(buffer, buffer.pointer) }};\
           drop(buffer);\
           view == \"hello\"\
         }}"
    ));
    fixture.check_with_destructors(&["TextBuffer"]);
    fixture.expect("E5002", "because it is borrowed");
}

#[test]
fn a_static_owner_can_lend_a_view() {
    check_clean(&format!(
        "{BUFFER}\
         static VALUE: i32 = 41;\
         static BUFFER: Buffer = Buffer {{ pointer: &VALUE as *raw i32 }};\
         func view() -> *i32 {{\
           unsafe {{ owned_view(BUFFER, BUFFER.pointer) }}\
         }}\
         func main() -> i32 {{ *view() }}"
    ));
}
