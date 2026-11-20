//! LLVM declarations for external functions and statics.
//!
//! These tests inspect the module before target-machine emission, so they can
//! assert the exact declaration text: the linker symbol, the C signature, the
//! absence of attributes, and the mutability of an external global.

use inkwell::context::Context;

use crate::tests::support::{build_module, front_end};

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
