//! External function and static type checking.
//!
//! The tests drive the whole front end: symbol validation happens during
//! lowering, so a fixture that only ran the type checker would miss `E3080`,
//! `E3082`, and `E3083`.

use super::support::{check_clean, check_errors};

#[test]
fn an_external_function_call_returns_its_declared_type() {
    let source = r#"
        extern "c" { func abs(value: i32) -> i32; }
        func main() -> i32 { unsafe { abs(-41) } }"#;
    check_clean(source);
}

#[test]
fn a_void_external_function_produces_unit() {
    let source = r#"
        extern "c" { func srand(seed: u32); }
        func main() { unsafe { srand(7u32); } }"#;
    check_clean(source);
}

#[test]
fn calling_an_external_function_requires_unsafe() {
    let source = r#"
        extern "c" { func abs(value: i32) -> i32; }
        func main() -> i32 { abs(-41) }"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3070", "requires an `unsafe` block");
}

#[test]
fn reading_an_external_static_requires_unsafe() {
    let source = r#"
        extern "c" { static value: i32; }
        func main() -> i32 { value }"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3086", "external static `value`");
}

#[test]
fn external_statics_can_be_read_in_unsafe() {
    let source = r#"
        extern "c" { static value: i32; }
        func main() -> i32 { unsafe { value } }"#;
    check_clean(source);
}

#[test]
fn an_immutable_external_static_cannot_be_written() {
    let source = r#"
        extern "c" { static value: i32; }
        func main() { unsafe { value = 1; } }"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3086", "cannot assign to the external static");
}

#[test]
fn a_mutable_external_static_can_be_written_in_unsafe() {
    let source = r#"
        extern "c" { static mut counter: i32; }
        func main() -> i32 { unsafe { counter = 1; counter } }"#;
    check_clean(source);
}

#[test]
fn a_checked_reference_parameter_is_rejected() {
    let source = r#"
        extern "c" { func read(pointer: *mut c_int); }
        func main() {}"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3084", "exclusive reference is not a C pointer");
}

#[test]
fn an_explicit_unit_return_is_void() {
    let source = r#"
        extern "c" { func srand(seed: u32) -> (); }
        func main() { unsafe { srand(7u32); } }"#;
    check_clean(source);
}

#[test]
fn a_non_ffi_safe_parameter_is_rejected() {
    let source = r#"
        struct Point { x: i32 }
        extern "c" { func take(point: Point); }
        func main() {}"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3084", "not FFI-safe");
}

#[test]
fn an_unknown_abi_is_rejected() {
    let source = r#"
        extern "stdcall" { func abs(value: i32) -> i32; }
        func main() {}"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3080", "unknown or unavailable ABI");
}

#[test]
fn an_invalid_symbol_is_rejected() {
    let source = r#"
        extern "c" { func abs(value: i32) -> i32 link_name ""; }
        func main() {}"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3082", "cannot be empty");
}

#[test]
fn one_symbol_cannot_have_two_shapes() {
    let source = r#"
        extern "c" {
            func first(value: i32) -> i32 link_name "shared";
            func second(value: i64) -> i64 link_name "shared";
        }
        func main() {}"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3083", "declared with two different types");
}

#[test]
fn two_declarations_cannot_share_a_seal_name() {
    let source = r#"
        extern "c" { func abs(value: i32) -> i32; func abs(value: i64) -> i64; }
        func main() {}"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E2001", "defined twice");
}

#[test]
fn declaring_the_same_symbol_with_one_shape_is_accepted() {
    let source = r#"
        extern "c" {
            func first(value: i32) -> i32 link_name "shared";
            func second(value: i32) -> i32 link_name "shared";
        }
        func main() {}"#;
    check_clean(source);
}
