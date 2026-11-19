//! End-to-end behavior of opt-in `Copy` values.
//!
//! Every runtime scenario compiles the real program at `-O0` and `-O2`; the
//! final section inspects assembly to prove that a `Copy` ADT needs no drop
//! glue.

use std::fs;

use seal_codegen::Emit;
use seal_session::OptLevel;

mod support;

use support::{TempOutput, compile, run};

const POINT: &str = "struct Point { x: i32, y: i32 } impl Copy for Point {}";

#[test]
fn two_copies_of_a_struct_are_independent() {
    let source = format!(
        "{POINT} \
         func main() -> i32 {{ \
           let mut first = Point {{ x: 40, y: 1 }}; \
           let second = first; \
           first.x = 1; \
           second.x + first.x + second.y \
         }}"
    );
    assert_returns(&source, 42);
}

#[test]
fn a_copy_enum_is_usable_after_a_by_value_match() {
    assert_returns(
        "enum Message { Empty, Value(i32) } \
         impl Copy for Message {} \
         func score(message: Message) -> i32 { \
           match message { Message::Empty => 0, Message::Value(value) => value } \
         } \
         func main() -> i32 { \
           let message = Message::Value(21); \
           let copy = message; \
           score(message) + score(copy) \
         }",
        42,
    );
}

#[test]
fn a_copy_source_stays_usable_after_a_by_value_for_loop() {
    let source = format!(
        "{POINT} \
         func main() -> i32 {{ \
           let points = [Point {{ x: 20, y: 0 }}, Point {{ x: 1, y: 0 }}]; \
           let mut total = 0; \
           for point in points {{ total += point.x; }} \
           total + points[0].x + points[1].x \
         }}"
    );
    assert_returns(&source, 42);
}

#[test]
fn dropping_a_copy_value_does_not_change_it() {
    let source = format!(
        "{POINT} \
         func main() -> i32 {{ \
           let point = Point {{ x: 42, y: 0 }}; \
           drop(point); \
           point.x \
         }}"
    );
    assert_returns(&source, 42);
}

#[test]
fn a_copy_static_is_readable_by_value() {
    let source = format!(
        "{POINT} \
         static HOME: Point = Point {{ x: 40, y: 2 }}; \
         func main() -> i32 {{ \
           let home = HOME; \
           home.x + home.y \
         }}"
    );
    assert_returns(&source, 42);
}

#[test]
fn a_copy_adt_emits_no_drop_glue() {
    let source = format!(
        "{POINT} \
         func main() -> i32 {{ \
           let point = Point {{ x: 42, y: 0 }}; \
           drop(point); \
           point.x \
         }}"
    );
    let output = TempOutput::new("copy-no-glue");
    compile(&source, output.path(), Emit::Assembly, OptLevel::O0, false)
        .expect("the program emits assembly");
    let assembly = fs::read_to_string(output.path()).expect("the assembly is readable");
    assert!(
        !assembly.contains("seal.drop.glue."),
        "a Copy ADT must not emit drop glue:\n{assembly}"
    );
    assert!(
        !assembly.contains("seal.drop.noop"),
        "a Copy ADT must not emit a no-op glue:\n{assembly}"
    );
}

/// Compiles `source` at both optimization levels and asserts the exit status.
fn assert_returns(source: &str, expected: i32) {
    for opt_level in [OptLevel::O0, OptLevel::O2] {
        let output = TempOutput::new("copy-optin");
        compile(source, output.path(), Emit::Executable, opt_level, false)
            .unwrap_or_else(|error| panic!("the program failed at {opt_level:?}: {error}"));
        assert_eq!(
            run(output.path()).code(),
            Some(expected),
            "the program returned the wrong value at {opt_level:?}"
        );
    }
}
