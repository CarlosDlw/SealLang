//! Command-line regression fixtures for global-data diagnostics.
//!
//! Each case pins the code, the message, and the exit status through the same
//! binary a user runs, so a change in rendering or phase ordering is caught
//! here even when the crate-level test still passes.

mod support;

use support::reject;

#[test]
fn a_value_declaration_needs_an_explicit_type() {
    let output = reject(
        "global-missing-type",
        "const LIMIT = 3;\nfunc main() -> i32 { 0 }\n",
        "E1012",
        "`const` declaration needs an explicit type",
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("write `const NAME: Type = value;`"),
        "the help shows the required shape: {stderr}"
    );
    assert!(
        stderr.contains(":1:7"),
        "the error points at the declaration name: {stderr}"
    );
}

#[test]
fn a_reserved_keyword_names_the_reserved_word() {
    let output = reject(
        "global-reserved-keyword",
        "const unsafe: i32 = 1;\nfunc main() -> i32 { 0 }\n",
        "E1005",
        "is a reserved keyword",
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("rename the identifier"),
        "the help explains the migration step: {stderr}"
    );
}

#[test]
fn moving_a_non_copy_value_out_of_a_static_is_rejected() {
    reject(
        "global-move-out-of-static",
        "struct Point { x: i32, y: i32 }\n\
         static HOME: Point = Point { x: 0, y: 0 };\n\
         func main() -> Point { HOME }\n",
        "E3058",
        "cannot move `HOME` out of a static",
    );
}

#[test]
fn writing_an_immutable_static_is_rejected() {
    reject(
        "global-write-immutable",
        "static LIMIT: i32 = 3;\nfunc main() { LIMIT = 4; }\n",
        "E3059",
        "immutable static `LIMIT`",
    );
}

#[test]
fn reading_a_mutable_static_requires_unsafe() {
    reject(
        "global-mutable-read",
        "static mut COUNTER: i32 = 0;\nfunc main() -> i32 { COUNTER }\n",
        "E3063",
        "requires an `unsafe` block",
    );
}

#[test]
fn writing_a_mutable_static_requires_unsafe() {
    reject(
        "global-mutable-write",
        "static mut COUNTER: i32 = 0;\nfunc main() { COUNTER = 1; }\n",
        "E3063",
        "requires an `unsafe` block",
    );
}

#[test]
fn a_constant_initializer_cannot_call_a_non_const_function() {
    reject(
        "global-non-const-call",
        "func build() -> i32 { 1 }\nconst VALUE: i32 = build();\nfunc main() -> i32 { 0 }\n",
        "E4011",
        "cannot call a non-`const` function",
    );
}

#[test]
fn a_constant_initializer_cannot_read_a_mutable_static() {
    reject(
        "global-const-reads-mutable-static",
        "static mut COUNTER: i32 = 0;\n\
         const VALUE: i32 = unsafe { COUNTER };\n\
         func main() -> i32 { 0 }\n",
        "E4026",
        "cannot read a mutable static",
    );
}

#[test]
fn a_constant_cannot_contain_a_mutable_reference() {
    reject(
        "global-const-mutable-reference",
        "static mut VALUE: i32 = 0;\nstatic POINTER: *mut i32 = &mut VALUE;\n\
         func main() -> i32 { 0 }\n",
        "E4012",
        "cannot contain a mutable reference",
    );
}

#[test]
fn division_by_zero_in_a_constant_is_reported_with_its_item() {
    let output = reject(
        "global-const-division-by-zero",
        "const VALUE: i32 = 7 / 0;\nfunc main() -> i32 { 0 }\n",
        "E4002",
        "division by zero in a constant expression",
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(":1:20"),
        "the error points at the division: {stderr}"
    );
    assert!(
        stderr.contains("while evaluating `VALUE`"),
        "the note names the evaluated item: {stderr}"
    );
}

#[test]
fn a_constant_cycle_names_both_items() {
    reject(
        "global-const-cycle",
        "const A: i32 = B;\nconst B: i32 = A;\nfunc main() -> i32 { 0 }\n",
        "E4007",
        "constant evaluation cycle through value dependencies: A -> B -> A",
    );
}

#[test]
fn a_call_that_writes_a_borrowed_static_is_rejected() {
    reject(
        "global-call-writes-borrowed-static",
        "static mut COUNTER: i32 = 0;\n\
         func reset() {\n  unsafe { COUNTER = 0; }\n}\n\
         func broken() -> i32 {\n  unsafe {\n    let value = &mut COUNTER;\n    reset();\n    *value\n  }\n}\n",
        "E5013",
        "may write the mutable static `COUNTER`",
    );
}

#[test]
fn a_reference_to_a_mutable_static_cannot_escape() {
    reject(
        "global-mutable-static-escape",
        "static mut COUNTER: i32 = 0;\n\
         func leak() -> *mut i32 {\n  unsafe { &mut COUNTER }\n}\n\
         func main() -> i32 { 0 }\n",
        "E5012",
        "mutable static `COUNTER`",
    );
}
