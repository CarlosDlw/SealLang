//! LLVM declarations for external functions and statics.
//!
//! These tests inspect the module before target-machine emission, so they can
//! assert the exact declaration text: the linker symbol, the C signature, the
//! absence of attributes, and the mutability of an external global.

use inkwell::context::Context;

use crate::tests::support::{build_module, front_end, lowered_debug_module_ir};

/// Lowers `source` and returns the IR text of the resulting module.
fn ir_text(source: &str) -> String {
    let compiled = front_end(source);
    let context = Context::create();
    let module = build_module(&compiled, &context, false);
    module.print_to_string().to_string()
}

/// Returns the declaration line for `symbol`.
fn declaration<'a>(ir: &'a str, symbol: &str) -> &'a str {
    ir.lines()
        .find(|line| line.starts_with("declare ") && line.contains(symbol))
        .unwrap_or_else(|| panic!("no declaration for `{symbol}` in:\n{ir}"))
}

#[test]
fn an_external_function_declares_the_c_signature_without_attributes() {
    let ir = ir_text(
        r#"extern "c" { func abs(value: i32) -> i32; }
           func main() -> i32 { unsafe { abs(-41) } }"#,
    );
    assert_eq!(declaration(&ir, "@abs("), "declare i32 @abs(i32)");
    assert!(
        ir.lines().any(|line| line.contains("call i32 @abs(")),
        "the call uses the C signature:\n{ir}"
    );
}

#[test]
fn a_link_name_is_the_declared_symbol() {
    let ir = ir_text(
        r#"extern "c" { func absolute(value: i32) -> i32 link_name "abs"; }
           func main() -> i32 { unsafe { absolute(-41) } }"#,
    );
    assert_eq!(declaration(&ir, "@abs("), "declare i32 @abs(i32)");
    assert!(
        !ir.contains("@absolute"),
        "the Seal name never reaches the object file:\n{ir}"
    );
}

#[test]
fn an_external_static_is_an_external_mutable_global() {
    let ir = ir_text(
        r#"extern "c" { static mut environ: *raw mut *raw mut c_char; }
           func main() -> i32 { unsafe { environ as usize as i32 } }"#,
    );
    let global = ir
        .lines()
        .find(|line| line.starts_with("@environ"))
        .unwrap_or_else(|| panic!("no global for `environ` in:\n{ir}"));
    assert!(
        global.contains("external global"),
        "the global has external linkage: {global}"
    );
    assert!(
        !global.contains("constant"),
        "C owns the storage, so the global is not constant: {global}"
    );
}

#[test]
fn a_small_c_struct_is_coerced_to_its_abi_pieces() {
    let ir = ir_text(
        r#"extern struct Point { x: c_int, y: c_int }
           extern "c" { func ffi_point_new(x: c_int, y: c_int) -> Point; }
           func main() -> i32 { unsafe { let p = ffi_point_new(1, 2); p.x } }"#,
    );
    assert_eq!(
        declaration(&ir, "@ffi_point_new("),
        "declare i64 @ffi_point_new(i32, i32)"
    );
    assert!(
        ir.lines()
            .any(|line| line.contains("call i64 @ffi_point_new(")),
        "the call uses the coerced return:\n{ir}"
    );
}

#[test]
fn a_large_c_struct_is_returned_through_hidden_storage() {
    let ir = ir_text(
        r#"extern struct Wide { first: c_int, second: f64, third: i64 }
           extern "c" { func ffi_wide() -> Wide; }
           func main() -> i32 { unsafe { let wide = ffi_wide(); wide.first } }"#,
    );
    let declaration = declaration(&ir, "@ffi_wide(");
    assert_eq!(
        declaration,
        "declare void @ffi_wide(ptr sret(%seal.adt.0.Wide))"
    );
    assert!(
        ir.lines()
            .any(|line| line.contains("call void @ffi_wide(ptr")),
        "the call passes the hidden storage pointer:\n{ir}"
    );
}

#[test]
fn a_large_c_struct_parameter_is_passed_byval() {
    let ir = ir_text(
        r#"extern struct Wide { first: c_int, second: f64, third: i64 }
           extern "c" { func ffi_take(wide: Wide); }
           func main() { unsafe { let wide = Wide { first: 1, second: 2.0, third: 3 }; ffi_take(wide); } }"#,
    );
    let declaration = declaration(&ir, "@ffi_take(");
    assert!(
        declaration.contains("byval(%seal.adt.0.Wide)"),
        "the parameter is copied to the stack: {declaration}"
    );
}

#[test]
fn a_variadic_declaration_and_call_use_the_c_defaults() {
    let ir = ir_text(
        r#"extern "c" { func printf(format: *raw u8, ...) -> i32; }
           func main() -> i32 {
               unsafe { printf(c"%d", 1i8, 2u16, true, 1.5f32, 3i64) }
           }"#,
    );
    assert_eq!(
        declaration(&ir, "@printf("),
        "declare i32 @printf(ptr, ...)"
    );
    let call = ir
        .lines()
        .find(|line| line.contains("call i32") && line.contains("@printf("))
        .unwrap_or_else(|| panic!("no call to `printf` in:\n{ir}"));
    assert!(
        call.contains("i32 (ptr, ...) @printf("),
        "the call uses the variadic signature: {call}"
    );
    assert!(
        call.contains("ptr @seal.str.0, i32") && call.matches(", i32").count() == 3,
        "narrow integers and bool pass as `i32`: {call}"
    );
    assert!(
        call.contains(", double ") && call.contains(", i64 3)"),
        "`f32` widens to `f64` and `i64` keeps its width: {call}"
    );
}

#[test]
fn a_c_string_global_is_nul_terminated() {
    let ir = ir_text(
        r#"extern "c" { func ffi_cstr_len(text: *raw u8) -> i32; }
           func main() -> i32 { unsafe { ffi_cstr_len(c"seal") } }"#,
    );
    let global = ir
        .lines()
        .find(|line| line.contains("@seal.str.") && line.contains("constant"))
        .unwrap_or_else(|| panic!("no string global in:\n{ir}"));
    assert!(
        global.contains("c\"seal\\00\""),
        "the global carries the terminator: {global}"
    );
}

#[test]
fn debug_information_describes_a_c_struct_with_its_layout() {
    let compiled = front_end(
        r#"extern struct Point { x: c_int, y: c_int }
           extern "c" { func ffi_take(point: Point); }
           func main() { unsafe { ffi_take(Point { x: 1, y: 2 }); } }"#,
    );
    let ir = lowered_debug_module_ir(&compiled);
    let aggregate = ir
        .lines()
        .find(|line| line.contains("DICompositeType") && line.contains("name: \"Point\""))
        .unwrap_or_else(|| panic!("no debug record for Point:\n{ir}"));
    assert!(
        aggregate.contains("size: 64"),
        "the debug record carries the C size: {aggregate}"
    );
}

#[test]
fn a_program_without_extern_declarations_has_no_external_declarations() {
    let source = "func main() -> i32 { 0 }";
    let ir = ir_text(source);
    assert!(
        !ir.lines().any(|line| line.starts_with("declare ")),
        "a program without extern emits no declarations:\n{ir}"
    );
    assert!(
        ir.contains("define i32 @seal.main()"),
        "the entry point is still defined:\n{ir}"
    );
    assert_eq!(ir, ir_text(source), "IR generation is deterministic");
}
