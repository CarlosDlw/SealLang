//! Reference parameter attribute tests.
//!
//! `readonly` follows from the type system, while `noalias` is an optimization
//! gated by borrow checking's facts. These tests inspect the declared LLVM
//! functions directly instead of running the program.

use super::support::{front_end, module_ir};

/// Runs the front end and returns the IR of every declared function.
fn ir_of(source: &str) -> String {
    module_ir(&front_end(source))
}

#[test]
fn an_exclusive_parameter_is_noalias_when_the_proof_closes() {
    let ir = ir_of(
        "func bump(count: *mut i32) {\n  *count = *count + 1;\n}\n\n\
         func main() -> i32 {\n  let mut value = 1;\n  bump(&mut value);\n  value\n}\n",
    );
    assert!(ir.contains("noalias"), "expected `noalias` in:\n{ir}");
}

#[test]
fn a_shared_parameter_is_readonly() {
    let ir = ir_of(
        "func sum(a: *i32, b: *i32) -> i32 {\n  *a + *b\n}\n\n\
         func main() -> i32 {\n  let x = 21;\n  let r = &x;\n  sum(r, r)\n}\n",
    );
    assert!(ir.contains("readonly"), "expected `readonly` in:\n{ir}");
    assert!(
        !ir.contains("noalias"),
        "shared references are not exclusive: {ir}"
    );
}

#[test]
fn a_fat_pointer_parameter_keeps_the_ordinary_abi() {
    let ir = ir_of(
        "func show(text: *str) {}\n\n\
         func main() -> i32 {\n  show(\"seal\");\n  0\n}\n",
    );
    assert!(!ir.contains("readonly"), "unexpected attribute in:\n{ir}");
}

#[test]
fn an_unknown_pointer_in_the_callee_withholds_noalias() {
    let ir = ir_of(
        "struct Node {\n  next: *mut i32,\n}\n\n\
         func follow(node: *mut Node) -> i32 {\n  let next = (*node).next;\n  *next\n}\n\n\
         func main() -> i32 {\n  let mut value = 7;\n  let mut node = Node { next: &mut value };\n  follow(&mut node)\n}\n",
    );
    assert!(!ir.contains("noalias"), "unexpected `noalias` in:\n{ir}");
}

#[test]
fn an_unproven_call_argument_withholds_noalias() {
    let ir = ir_of(
        "struct Node {\n  next: *mut i32,\n}\n\n\
         func bump(count: *mut i32) {\n  *count = *count + 1;\n}\n\n\
         func main() -> i32 {\n  let mut value = 7;\n  let mut node = Node { next: &mut value };\n  let r = &node;\n  bump((*r).next);\n  0\n}\n",
    );
    assert!(!ir.contains("noalias"), "unexpected `noalias` in:\n{ir}");
}
