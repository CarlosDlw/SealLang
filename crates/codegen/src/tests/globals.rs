//! IR-level tests for global declarations and constant relocations.
//!
//! The integration tests run programs and inspect artifacts; these tests assert
//! the exact IR shape a global needs: linkage, mutability, and the constant
//! expression that names another global.

use super::support::{front_end, lowered_debug_module_ir, lowered_module_ir};

/// Lowers `source` and returns its complete IR without debug information.
fn ir_of(source: &str) -> String {
    lowered_module_ir(&front_end(source))
}

#[test]
fn an_immutable_static_is_an_internal_constant() {
    let ir = ir_of("static LIMIT: i32 = 40;\nfunc main() -> i32 { LIMIT }");
    assert!(
        ir.contains("@seal.static.LIMIT = internal constant"),
        "an immutable static must be constant and internal:\n{ir}"
    );
}

#[test]
fn a_mutable_static_is_an_internal_writable_global() {
    let ir = ir_of("static mut COUNTER: i32 = 7;\nfunc main() -> i32 { unsafe { COUNTER } }");
    assert!(
        ir.contains("@seal.static.COUNTER = internal global"),
        "a mutable static must be writable:\n{ir}"
    );
}

#[test]
fn a_string_static_relocates_the_literal_and_its_length() {
    let ir = ir_of("static NAME: *str = \"seal\";\nfunc main() -> i32 { let name = NAME; 0 }");
    assert!(
        ir.contains("@seal.static.NAME = internal constant <{ ptr, [8 x i8] }>"),
        "a string static is a relocated pointer followed by its length:\n{ir}"
    );
    assert!(
        ir.contains("ptr @seal.str."),
        "the data pointer is a relocation to the string literal:\n{ir}"
    );
    assert!(
        ir.contains(r#"c"\04\00\00\00\00\00\00\00""#),
        "the length travels with the pointer:\n{ir}"
    );
}

#[test]
fn an_addressed_const_is_private_and_relocates_with_a_byte_offset() {
    let ir = ir_of(
        "const TABLE: [i32; 3] = [7, 8, 9];\n\
         static SECOND: *i32 = &TABLE[1];\n\
         func main() -> i32 { *SECOND }",
    );
    assert!(
        ir.contains("private constant"),
        "an addressed const allocation is private:\n{ir}"
    );
    assert!(
        ir.contains("getelementptr") && ir.contains("i64 4"),
        "the reference is relocated to the element's byte offset:\n{ir}"
    );
}

#[test]
fn a_value_constant_is_read_from_a_private_global() {
    let ir = ir_of(
        "struct Pair {\n  value: i32,\n  text: *str,\n}\n\
         const PAIR: Pair = Pair { value: 7, text: \"x\" };\n\
         func main() -> i32 { let pair = PAIR; pair.value }",
    );
    assert!(
        ir.contains("@seal.value.PAIR = private constant"),
        "a value constant is copied from a private global:\n{ir}"
    );
}

#[test]
fn a_static_trait_object_relocates_its_data_and_vtable() {
    let ir = ir_of(
        "trait Area {\n  func area(self: *Self) -> i32;\n}\n\n\
         struct Square {\n  side: i32,\n}\n\n\
         impl Area for Square {\n  func area(self: *Square) -> i32 {\n    (*self).side\n  }\n}\n\n\
         static SQUARE: Square = Square { side: 3 };\n\
         static SHAPE: *dyn Area = &SQUARE;\n\
         func main() -> i32 { SHAPE.area() }",
    );
    assert!(
        ir.contains("@seal.static.SHAPE = internal constant <{ ptr, ptr }>"),
        "a trait object is a data pointer and a vtable pointer:\n{ir}"
    );
    assert!(
        ir.contains("ptr @seal.static.SQUARE, ptr @"),
        "the data pointer names the static and the vtable is a relocation:\n{ir}"
    );
}

#[test]
fn debug_information_describes_statics_and_local_statics() {
    let ir = lowered_debug_module_ir(&front_end(
        "static LIMIT: i32 = 40;\n\n\
         func next() -> i32 {\n  static mut CALLS: i32 = 0;\n  unsafe {\n    CALLS = CALLS + 1;\n  }\n  unsafe { CALLS }\n}\n\n\
         func main() -> i32 { LIMIT + next() }",
    ));
    assert!(
        ir.contains("DIGlobalVariable(name: \"LIMIT\""),
        "the module static has no debug record:\n{ir}"
    );
    assert!(
        ir.contains("DIGlobalVariable(name: \"CALLS\""),
        "the local static has no debug record:\n{ir}"
    );
    assert!(
        ir.contains("@seal.static.LIMIT = internal constant") && ir.contains(", !dbg !"),
        "the debug record is not attached to the global:\n{ir}"
    );
}
