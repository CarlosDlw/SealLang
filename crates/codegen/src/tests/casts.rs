//! LLVM lowering of explicit numeric casts.
//!
//! These tests inspect the emitted IR directly: the conversion instruction
//! must match the source and target types, and float-to-integer must go
//! through the saturating intrinsic rather than an undefined plain conversion.

use super::support::{front_end, function_ir, lowered_module_ir};

fn ir_of(source: &str) -> String {
    lowered_module_ir(&front_end(source))
}

fn body_of(source: &str, name: &str) -> String {
    function_ir(&ir_of(source), &format!("seal.{name}"))
}

#[test]
fn a_signed_widening_cast_sign_extends() {
    let function = body_of("func widen(value: i32) -> i64 { value as i64 }", "widen");
    assert!(
        function.contains("sext i32"),
        "expected a sign extension in:\n{function}"
    );
}

#[test]
fn an_unsigned_widening_cast_zero_extends() {
    let function = body_of("func widen(value: u32) -> u64 { value as u64 }", "widen");
    assert!(
        function.contains("zext i32"),
        "expected a zero extension in:\n{function}"
    );
}

#[test]
fn a_narrowing_cast_truncates() {
    let function = body_of("func narrow(value: i64) -> u8 { value as u8 }", "narrow");
    assert!(
        function.contains("trunc i64"),
        "expected a truncation in:\n{function}"
    );
}

#[test]
fn a_same_width_cast_emits_no_conversion() {
    let function = body_of("func same(value: i32) -> u32 { value as u32 }", "same");
    assert!(
        !function.contains("sext") && !function.contains("zext") && !function.contains("trunc"),
        "a same-width cast is a no-op:\n{function}"
    );
}

#[test]
fn an_integer_to_float_cast_preserves_signedness() {
    let signed = body_of("func widen(value: i32) -> f64 { value as f64 }", "widen");
    assert!(
        signed.contains("sitofp i32"),
        "expected `sitofp` in:\n{signed}"
    );
    let unsigned = body_of("func widen(value: u32) -> f32 { value as f32 }", "widen");
    assert!(
        unsigned.contains("uitofp i32"),
        "expected `uitofp` in:\n{unsigned}"
    );
}

#[test]
fn a_float_to_float_cast_uses_the_right_instruction() {
    let widen = body_of("func widen(value: f32) -> f64 { value as f64 }", "widen");
    assert!(
        widen.contains("fpext float"),
        "expected `fpext` in:\n{widen}"
    );
    let narrow = body_of("func narrow(value: f64) -> f32 { value as f32 }", "narrow");
    assert!(
        narrow.contains("fptrunc double"),
        "expected `fptrunc` in:\n{narrow}"
    );
}

#[test]
fn a_float_to_integer_cast_uses_the_saturating_intrinsic() {
    let signed = body_of(
        "func convert(value: f64) -> i32 { value as i32 }",
        "convert",
    );
    assert!(
        signed.contains("llvm.fptosi.sat.i32.f64"),
        "expected the saturating signed intrinsic in:\n{signed}"
    );
    assert!(
        !signed.contains("fptosi double"),
        "a plain `fptosi` is undefined out of range:\n{signed}"
    );

    let unsigned = body_of(
        "func convert(value: f32) -> u64 { value as u64 }",
        "convert",
    );
    assert!(
        unsigned.contains("llvm.fptoui.sat.i64.f32"),
        "expected the saturating unsigned intrinsic in:\n{unsigned}"
    );
    assert!(
        !unsigned.contains("fptoui float"),
        "a plain `fptoui` is undefined out of range:\n{unsigned}"
    );
}

#[test]
fn a_cast_emits_no_runtime_call_or_allocation() {
    let ir = ir_of(
        "func convert(value: i32) -> f64 { value as f64 } \
         func main() -> i32 { convert(1) as i32 }",
    );
    assert!(
        !ir.contains("malloc") && !ir.contains("@seal_rt"),
        "a cast allocates nothing and calls no runtime helper:\n{ir}"
    );
}
