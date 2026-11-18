//! Compile-time `const func` calls and the intrinsic whitelist.

mod support;

use seal_const_eval::{ConstValue, EvalLimits};
use support::{Fixture, clean};

fn int(fixture: &Fixture, name: &str) -> u128 {
    match fixture.value(name) {
        ConstValue::Int { bits, .. } => *bits,
        other => panic!("`{name}` is not an integer: {other:?}"),
    }
}

#[test]
fn a_const_function_returns_its_value() {
    let fixture = clean(
        "const func twice(value: i32) -> i32 { value * 2 } \
         const RESULT: i32 = twice(21);",
    );
    assert_eq!(int(&fixture, "RESULT"), 42);
}

#[test]
fn const_functions_call_each_other() {
    let fixture = clean(
        "const func increment(value: i32) -> i32 { value + 1 } \
         const func twice(value: i32) -> i32 { increment(increment(value)) } \
         const RESULT: i32 = twice(40);",
    );
    assert_eq!(int(&fixture, "RESULT"), 42);
}

#[test]
fn recursion_is_bounded_and_terminating() {
    let fixture = clean(
        "const func factorial(value: u32) -> u32 { \
             if value <= 1 { 1 } else { value * factorial(value - 1) } \
         } \
         const RESULT: u32 = factorial(5);",
    );
    assert_eq!(int(&fixture, "RESULT"), 120);
}

#[test]
fn const_functions_read_immutable_statics() {
    let fixture = clean(
        "static LIMIT: i32 = 40; \
         const func add_limit(value: i32) -> i32 { value + LIMIT } \
         const RESULT: i32 = add_limit(2);",
    );
    assert_eq!(int(&fixture, "RESULT"), 42);
}

#[test]
fn a_reference_to_a_callee_local_cannot_escape() {
    let fixture = Fixture::compile(
        "const func pointer() -> *i32 { let value = 1; &value } \
         const RESULT: *i32 = pointer();",
    );
    fixture.expect_code("E4013");
}

#[test]
fn const_functions_agree_with_direct_expressions() {
    let fixture = clean(
        "const func add(left: i32, right: i32) -> i32 { left + right } \
         const func magnitude(value: i32) -> i32 { value.abs() } \
         const THROUGH_CALL: i32 = add(magnitude(-40), 2); \
         const DIRECT: i32 = ((-40i32).abs() + 2);",
    );
    assert_eq!(int(&fixture, "THROUGH_CALL"), int(&fixture, "DIRECT"));
}

#[test]
fn generic_calls_instantiate_the_body() {
    let fixture = clean(
        "const func identity<T>(value: T) -> T { value } \
         const RESULT: i32 = identity::<i32>(42);",
    );
    assert_eq!(int(&fixture, "RESULT"), 42);
}

#[test]
fn compile_time_recursion_hits_the_depth_limit() {
    let limits = EvalLimits {
        call_depth: 8,
        steps: 1_000,
        ..EvalLimits::default()
    };
    let fixture = Fixture::compile_with(
        "const func factorial(value: u32) -> u32 { \
             if value <= 1 { 1 } else { value * factorial(value - 1) } \
         } \
         const RESULT: u32 = factorial(20);",
        limits,
    );
    fixture.expect_code("E4009");
}

#[test]
fn a_compile_time_panic_is_an_error() {
    let fixture = Fixture::compile(
        "const func divide(value: i32) -> i32 { value / 0 } \
         const RESULT: i32 = divide(1);",
    );
    fixture.expect_code("E4002");
}

#[test]
fn sequence_intrinsics_read_and_mutate_local_arrays() {
    let fixture = clean(
        "const LEN: usize = { let values = [1, 2, 3]; values.len() }; \
         const EMPTY: bool = { let values: [i32; 0] = []; values.is_empty() }; \
         const REVERSED: i32 = { \
             let mut values = [1, 2, 3]; values.reverse(); \
             values[0] * 100 + values[1] * 10 + values[2] \
         }; \
         const SWAPPED: i32 = { \
             let mut values = [1, 2]; values.swap(0, 1); values[0] * 10 + values[1] \
         }; \
         const FILLED: i32 = { \
             let mut values = [0, 0, 0]; values.fill(7); \
             values[0] + values[1] + values[2] \
         };",
    );
    assert_eq!(int(&fixture, "LEN"), 3);
    assert_eq!(fixture.value("EMPTY"), &ConstValue::Bool(true));
    assert_eq!(int(&fixture, "REVERSED"), 321);
    assert_eq!(int(&fixture, "SWAPPED"), 21);
    assert_eq!(int(&fixture, "FILLED"), 21);
}

#[test]
fn integer_intrinsics_match_runtime_semantics() {
    let fixture = clean(
        "const ABS: i32 = (-5).abs(); \
         const WRAPPED: i8 = (-128i8).abs(); \
         const SIGN: i32 = (-9i32).signum(); \
         const MINIMUM: i32 = 4i32.min(2); \
         const MAXIMUM: u8 = 4u8.max(2); \
         const CLAMPED: i32 = 9i32.clamp(1, 5); \
         const WRAPPING: u8 = 250u8.wrapping_add(10); \
         const SATURATED: i8 = 120i8.saturating_add(40); \
         const UNDERFLOWED: u8 = 3u8.saturating_sub(5); \
         const SATURATED_UNSIGNED: u8 = 250u8.saturating_add(10); \
         const MULTIPLIED: i8 = 100i8.saturating_mul(2); \
         const ONES: u32 = 183u8.count_ones(); \
         const LEADING: u32 = 1u8.leading_zeros(); \
         const TRAILING: u32 = 8u8.trailing_zeros(); \
         const ROTATED: u8 = 129u8.rotate_left(1); \
         const ROTATED_RIGHT: u8 = 129u8.rotate_right(1); \
         const SWAPPED: u16 = 4660u16.swap_bytes();",
    );
    assert_eq!(int(&fixture, "ABS"), 5);
    assert_eq!(int(&fixture, "WRAPPED"), 128);
    assert_eq!(int(&fixture, "SIGN"), (-1i128 as u128) & 0xffff_ffff);
    assert_eq!(int(&fixture, "MINIMUM"), 2);
    assert_eq!(int(&fixture, "MAXIMUM"), 4);
    assert_eq!(int(&fixture, "CLAMPED"), 5);
    assert_eq!(int(&fixture, "WRAPPING"), 4);
    assert_eq!(int(&fixture, "SATURATED"), 127);
    assert_eq!(int(&fixture, "UNDERFLOWED"), 0);
    assert_eq!(int(&fixture, "SATURATED_UNSIGNED"), 255);
    assert_eq!(int(&fixture, "MULTIPLIED"), 127);
    assert_eq!(int(&fixture, "ONES"), 6);
    assert_eq!(int(&fixture, "LEADING"), 7);
    assert_eq!(int(&fixture, "TRAILING"), 3);
    assert_eq!(int(&fixture, "ROTATED"), 3);
    assert_eq!(int(&fixture, "ROTATED_RIGHT"), 0xc0);
    assert_eq!(int(&fixture, "SWAPPED"), 0x3412);
}

#[test]
fn float_intrinsics_follow_ieee_rules() {
    let fixture = clean(
        "const ABS: f64 = (-2.5).abs(); \
         const MINIMUM: f64 = 3.0.min(1.0); \
         const MAXIMUM: f64 = 3.0.max(1.0); \
         const CLAMPED: f64 = 9.0.clamp(1.0, 5.0); \
         const SQRT: f64 = 9.0.sqrt(); \
         const FLOOR: f64 = (-1.5).floor(); \
         const CEIL: f64 = (-1.5).ceil(); \
         const ROUNDED: f64 = 2.5.round(); \
         const TRUNCATED: f64 = (-1.9).trunc(); \
         const FRACTION: f64 = 2.75.fract(); \
         const POWER: f64 = 2.0.powi(10); \
         const NAN: bool = (0.0 / 0.0).is_nan(); \
         const FINITE: bool = 1.0.is_finite(); \
         const INFINITE: bool = (1.0 / 0.0).is_infinite();",
    );
    let float = |name: &str| match fixture.value(name) {
        ConstValue::Float { bits, .. } => f64::from_bits(*bits),
        other => panic!("`{name}` is not a float: {other:?}"),
    };
    assert_eq!(float("ABS"), 2.5);
    assert_eq!(float("MINIMUM"), 1.0);
    assert_eq!(float("MAXIMUM"), 3.0);
    assert_eq!(float("CLAMPED"), 5.0);
    assert_eq!(float("SQRT"), 3.0);
    assert_eq!(float("FLOOR"), -2.0);
    assert_eq!(float("CEIL"), -1.0);
    assert_eq!(float("ROUNDED"), 3.0);
    assert_eq!(float("TRUNCATED"), -1.0);
    assert_eq!(float("FRACTION"), 0.75);
    assert_eq!(float("POWER"), 1024.0);
    assert_eq!(fixture.value("NAN"), &ConstValue::Bool(true));
    assert_eq!(fixture.value("FINITE"), &ConstValue::Bool(true));
    assert_eq!(fixture.value("INFINITE"), &ConstValue::Bool(true));
}

#[test]
fn boolean_and_character_intrinsics() {
    let fixture = clean(
        "const NOT: bool = true.not(); \
         const NUMBER: i32 = true.to_i32(); \
         const TOGGLED: bool = { let mut flag = false; flag.toggle(); flag }; \
         const ASCII: bool = 'a'.is_ascii(); \
         const LETTER: bool = 'Z'.is_ascii_alphabetic(); \
         const DIGIT: bool = '7'.is_ascii_digit(); \
         const SPACE: bool = '\\t'.is_ascii_whitespace(); \
         const UPPER: char = 'q'.to_ascii_uppercase(); \
         const LOWER: char = 'Q'.to_ascii_lowercase(); \
         const BYTES: usize = 'é'.len_utf8(); \
         const VALUE: u32 = 'A'.to_u32();",
    );
    assert_eq!(fixture.value("NOT"), &ConstValue::Bool(false));
    assert_eq!(int(&fixture, "NUMBER"), 1);
    assert_eq!(fixture.value("TOGGLED"), &ConstValue::Bool(true));
    assert_eq!(fixture.value("ASCII"), &ConstValue::Bool(true));
    assert_eq!(fixture.value("LETTER"), &ConstValue::Bool(true));
    assert_eq!(fixture.value("DIGIT"), &ConstValue::Bool(true));
    assert_eq!(fixture.value("SPACE"), &ConstValue::Bool(true));
    assert_eq!(fixture.value("UPPER"), &ConstValue::Char('Q'));
    assert_eq!(fixture.value("LOWER"), &ConstValue::Char('q'));
    assert_eq!(int(&fixture, "BYTES"), 2);
    assert_eq!(int(&fixture, "VALUE"), 65);
}

#[test]
fn slice_methods_borrow_the_underlying_array() {
    let fixture = clean(
        "static VALUES: [i32; 3] = [1, 2, 3]; \
         const LEN: usize = { let slice: *[i32] = &VALUES[..]; slice.len() }; \
         const TAIL: usize = { let slice: *[i32] = &VALUES[1..]; slice.len() };",
    );
    assert_eq!(int(&fixture, "LEN"), 3);
    assert_eq!(int(&fixture, "TAIL"), 2);
}
