//! IR-level tests for pointer provenance.
//!
//! Internal field and index projects must be GEPs, never integer round trips.
//! Checked pointer arithmetic must be inbounds; wrapping arithmetic must keep
//! the pointer but drop the inbounds promise.

use super::support::{front_end, function_ir, lowered_module_ir};

fn ir_of(source: &str) -> String {
    lowered_module_ir(&front_end(source))
}

fn body_of(source: &str, name: &str) -> String {
    function_ir(&ir_of(source), &format!("seal.{name}"))
}

#[test]
fn field_and_index_projections_do_not_use_integer_arithmetic() {
    let body = body_of(
        "struct Pair { first: i32, second: i32 }\n\
         func choose(pair: Pair, values: [i32; 3]) -> i32 {\n\
             pair.second + values[1]\n\
         }\n\
         func main() -> i32 {\n\
             choose(Pair { first: 1, second: 41 }, [0, 1, 2])\n\
         }",
        "choose",
    );
    assert!(
        body.contains("getelementptr inbounds"),
        "field and index projects must be inbounds GEPs:\n{body}"
    );
    assert!(
        !body.contains("ptrtoint") && !body.contains("inttoptr"),
        "a projection must not convert between pointers and integers:\n{body}"
    );
}

#[test]
fn enum_field_projections_do_not_use_integer_arithmetic() {
    let body = body_of(
        "enum Message { Text(i32), Number(i32) }\n\
         func read(message: Message) -> i32 {\n\
             match message {\n\
                 Message::Text(value) => value,\n\
                 Message::Number(value) => value,\n\
             }\n\
         }\n\
         func main() -> i32 { read(Message::Number(42)) }",
        "read",
    );
    assert!(
        body.contains("getelementptr inbounds"),
        "an enum field must be reached by an inbounds GEP:\n{body}"
    );
    assert!(
        !body.contains("ptrtoint") && !body.contains("inttoptr"),
        "an enum field must not convert between pointers and integers:\n{body}"
    );
}

#[test]
fn checked_pointer_arithmetic_uses_inbounds_gep() {
    let body = body_of(
        "unsafe func advance(base: *raw i32) -> *raw i32 {\n\
             unsafe { base.add(2) }\n\
         }",
        "advance",
    );
    assert!(
        body.contains("getelementptr inbounds i32"),
        "`add` must preserve provenance with an inbounds GEP:\n{body}"
    );
    assert!(
        !body.contains("ptrtoint") && !body.contains("inttoptr"),
        "`add` must not convert between pointers and integers:\n{body}"
    );
}

#[test]
fn byte_pointer_arithmetic_uses_inbounds_gep() {
    let body = body_of(
        "unsafe func advance(base: *raw i32) -> *raw i32 {\n\
             unsafe { base.byte_add(4) }\n\
         }",
        "advance",
    );
    assert!(
        body.contains("getelementptr inbounds i8"),
        "`byte_add` must use a byte-wise inbounds GEP:\n{body}"
    );
    assert!(
        !body.contains("ptrtoint") && !body.contains("inttoptr"),
        "`byte_add` must not convert between pointers and integers:\n{body}"
    );
}

#[test]
fn wrapping_pointer_arithmetic_uses_gep_without_inbounds() {
    let body = body_of(
        "unsafe func advance(base: *raw i32) -> *raw i32 {\n\
             unsafe { base.wrapping_add(2) }\n\
         }",
        "advance",
    );
    assert!(
        body.contains("getelementptr i32"),
        "`wrapping_add` must still be a GEP:\n{body}"
    );
    assert!(
        !body.contains("getelementptr inbounds"),
        "`wrapping_add` must not promise inbounds:\n{body}"
    );
    assert!(
        !body.contains("ptrtoint") && !body.contains("inttoptr"),
        "`wrapping_add` must not convert between pointers and integers:\n{body}"
    );
}
