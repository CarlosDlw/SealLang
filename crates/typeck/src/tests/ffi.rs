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
    fixture.expect_help("E3084", "extern struct");
}

#[test]
fn an_extern_struct_crosses_the_boundary_by_value() {
    let source = r#"
        extern struct Point { x: c_int, y: c_int }
        extern "c" {
            func make(x: c_int, y: c_int) -> Point;
            func sum(point: Point) -> c_int;
        }
        func main() -> i32 {
            unsafe {
                let point = make(40, 2);
                sum(point)
            }
        }"#;
    check_clean(source);
}

#[test]
fn an_empty_extern_struct_is_rejected() {
    let source = r#"extern struct Empty {} func main() {}"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3085", "has no fields");
}

#[test]
fn a_unit_field_is_rejected() {
    let source = r#"extern struct Empty { value: () } func main() {}"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3085", "has no storage");
}

#[test]
fn a_bool_field_is_rejected_with_the_explicit_c_type() {
    let source = r#"extern struct Flags { enabled: bool } func main() {}"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3085", "cannot be `bool`");
    fixture.expect_help("E3085", "c_bool");
}

#[test]
fn a_checked_reference_field_is_rejected() {
    let source = r#"extern struct Holder { pointer: *mut c_int } func main() {}"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3084", "exclusive reference is not a C pointer");
}

#[test]
fn an_unsized_field_is_rejected() {
    let source = r#"extern struct Holder { text: str } func main() {}"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3084", "unsized");
}

#[test]
fn an_opaque_type_is_usable_behind_a_raw_pointer() {
    let source = r#"
        extern type FILE;
        extern "c" {
            func open(path: *raw u8) -> *raw mut FILE;
            func close(file: *raw mut FILE);
        }
        func main() {
            unsafe {
                let file = open(0usize as *raw u8);
                close(file);
            }
        }"#;
    check_clean(source);
}

#[test]
fn c_void_is_an_opaque_pointee() {
    let source = r#"
        extern "c" { func allocation() -> *raw mut c_void; }
        func main() { unsafe { let pointer = allocation(); } }"#;
    check_clean(source);
}

#[test]
fn an_opaque_parameter_and_return_are_rejected() {
    let source = r#"
        extern type FILE;
        func take(file: FILE) -> FILE { file }"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3087", "only appear behind a raw pointer");
}

#[test]
fn an_opaque_type_cannot_be_stored_by_value() {
    let source = r#"
        extern type FILE;
        func main() { let file: FILE; }"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3087", "only appear behind a raw pointer");
}

#[test]
fn c_void_cannot_be_stored_by_value() {
    let source = r#"func main() { let value: c_void; }"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3087", "only appear behind a raw pointer");
}

#[test]
fn an_opaque_field_is_rejected() {
    let source = r#"
        extern type FILE;
        extern struct Holder { file: FILE }
        func main() {}"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3087", "only appear behind a raw pointer");
}

#[test]
fn an_opaque_pointer_cannot_be_dereferenced() {
    let source = r#"
        extern type FILE;
        func read(file: *raw mut FILE) -> i32 { unsafe { *file } }"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3087", "cannot dereference the opaque type");
}

#[test]
fn an_extern_struct_can_implement_drop() {
    let source = r#"
        extern struct Buffer { data: *raw mut u8 }
        impl Drop for Buffer { func drop(self: *mut Buffer) {} }
        func main() {
            let buffer = Buffer { data: 0usize as *raw mut u8 };
            drop(buffer);
        }"#;
    check_clean(source);
}

#[test]
fn an_extern_struct_can_opt_into_copy() {
    let source = r#"
        extern struct Point { x: c_int, y: c_int }
        impl Copy for Point {}
        func main() {
            let point = Point { x: 1, y: 2 };
            let copy = point;
            let total = point.x + copy.y;
        }"#;
    check_clean(source);
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
fn an_external_function_named_main_is_rejected() {
    let source = r#"
        extern "c" { func main() -> c_int; }"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3082", "reserved for the entry point");
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
fn a_c_string_literal_has_a_const_u8_pointer_type() {
    let source = r#"
        func main() {
            let pointer: *raw u8 = c"hello";
            let _ = pointer;
        }"#;
    check_clean(source);
}

#[test]
fn a_variadic_call_accepts_promoted_arguments() {
    let source = r#"
        extern "c" { func log(format: *raw u8, ...) -> i32; }
        func main() -> i32 {
            unsafe { log(c"%d %.1f", 1i8, 2u16, true, 1.5f32, 4i64, 2.5f64) }
        }"#;
    check_clean(source);
}

#[test]
fn a_variadic_call_requires_every_fixed_argument() {
    let source = r#"
        extern "c" { func log(format: *raw u8, ...); }
        func main() { unsafe { log(); } }"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3008", "requires at least 1");
}

#[test]
fn a_str_cannot_cross_a_variadic_boundary() {
    let source = r#"
        extern "c" { func log(format: *raw u8, ...); }
        func main() {
            let text = "hello";
            unsafe { log(c"%s", text); }
        }"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3088", "cannot be passed through `...`");
}

#[test]
fn a_char_cannot_cross_a_variadic_boundary() {
    let source = r#"
        extern "c" { func log(format: *raw u8, ...); }
        func main() { unsafe { log(c"%c", 'x'); } }"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3088", "cannot be passed through `...`");
}

#[test]
fn an_extern_struct_cannot_cross_a_variadic_boundary() {
    let source = r#"
        extern struct Point { x: c_int }
        extern "c" { func log(format: *raw u8, ...); }
        func main() {
            let point = Point { x: 1 };
            unsafe { log(c"%p", point); }
        }"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3088", "cannot be passed through `...`");
}

#[test]
fn a_pointer_to_an_extern_struct_crosses_a_variadic_boundary() {
    let source = r#"
        extern struct Point { x: c_int }
        extern "c" { func log(format: *raw u8, ...); }
        func main() {
            let point = Point { x: 1 };
            let pointer = &point as *raw Point;
            unsafe { log(c"%p", pointer); }
        }"#;
    check_clean(source);
}

#[test]
fn a_c_function_pointer_can_be_taken_and_called_in_unsafe() {
    let source = r#"
        func double_it(value: i32) -> i32 { value * 2 }
        func apply(callback: *raw extern "c" func(i32) -> i32, value: i32) -> i32 {
            unsafe { callback(value) }
        }
        func main() -> i32 {
            unsafe {
                let callback = double_it as *raw extern "c" func(i32) -> i32;
                apply(callback, 21)
            }
        }"#;
    check_clean(source);
}

#[test]
fn taking_a_function_address_requires_unsafe() {
    let source = r#"
        func double_it(value: i32) -> i32 { value * 2 }
        func main() {
            let callback = double_it as *raw extern "c" func(i32) -> i32;
        }"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect(
        "E3089",
        "using a C function pointer requires an `unsafe` block",
    );
}

#[test]
fn calling_through_a_function_pointer_requires_unsafe() {
    let source = r#"
        func apply(callback: *raw extern "c" func(i32) -> i32, value: i32) -> i32 {
            callback(value)
        }"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect(
        "E3089",
        "using a C function pointer requires an `unsafe` block",
    );
}

#[test]
fn a_function_pointer_cast_requires_an_identical_signature() {
    let source = r#"
        func identity(value: i32) -> i32 { value }
        func main() {
            unsafe {
                let callback = identity as *raw extern "c" func(i64) -> i32;
            }
        }"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3089", "does not match the C function pointer type");
}

#[test]
fn a_method_path_is_rejected_before_it_can_become_a_c_function_pointer() {
    let source = r#"
        struct Counter { value: i32 }
        impl Counter {
            func bump(self: *mut Counter) {}
        }
        func main() {
            unsafe {
                let callback = Counter::bump as *raw extern "c" func();
            }
        }"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E2019", "is a method, not an associated function");
}

#[test]
fn an_integer_pointer_can_become_a_c_function_pointer() {
    let source = r#"
        func main() {
            unsafe {
                let pointer = c"seal" as *raw u8;
                let callback = pointer as *raw extern "c" func();
            }
        }"#;
    check_clean(source);
}

#[test]
fn a_generic_function_address_needs_explicit_arguments() {
    let source = r#"
        func identity<T>(value: T) -> T { value }
        func main() {
            unsafe {
                let callback = identity as *raw extern "c" func(i32) -> i32;
            }
        }"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3089", "needs explicit type arguments");
}

#[test]
fn an_explicitly_instantiated_generic_function_address_is_accepted() {
    let source = r#"
        func identity<T>(value: T) -> T { value }
        func main() {
            unsafe {
                let callback = identity::<i32> as *raw extern "c" func(i32) -> i32;
            }
        }"#;
    check_clean(source);
}

#[test]
fn a_mutable_c_function_pointer_is_rejected() {
    let source = r#"
        func take(callback: *raw mut extern "c" func()) {}"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3089", "cannot be `mut`");
}

#[test]
fn a_c_function_pointer_signature_is_checked_for_ffi_safety() {
    let source = r#"
        func take(callback: *raw extern "c" func(char)) {}"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3084", "is not FFI-safe");
}

#[test]
fn conflicting_variadic_shapes_are_rejected() {
    let source = r#"
        extern "c" {
            func first(format: *raw u8, ...) link_name "shared";
            func second(format: *raw u8) link_name "shared";
        }
        func main() {}"#;
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3083", "two different types");
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
