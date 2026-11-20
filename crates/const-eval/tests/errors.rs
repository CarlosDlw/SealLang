//! Failures reported by constant evaluation.

mod support;

use seal_const_eval::EvalLimits;
use support::Fixture;

#[test]
fn division_and_remainder_by_zero_are_reported() {
    let divide = Fixture::compile("const VALUE: i32 = 7 / 0;");
    divide.expect_code("E4002");
    let remainder = Fixture::compile("const VALUE: i32 = 7 % 0;");
    remainder.expect_code("E4003");
}

#[test]
fn dividing_the_signed_minimum_by_minus_one_is_reported() {
    let fixture = Fixture::compile("const VALUE: i32 = -2147483648 / -1;");
    fixture.expect_code("E4004");
}

#[test]
fn a_pointer_address_is_not_computable_in_a_constant() {
    let fixture = Fixture::compile(
        "static LIMIT: i32 = 7; \
         const POINTER: *raw i32 = &LIMIT as *raw i32; \
         const ADDRESS: usize = POINTER as usize;",
    );
    fixture.expect_code("E4030");
}

#[test]
fn a_constant_cannot_write_through_a_raw_pointer() {
    let fixture = Fixture::compile(
        "const POINTER: *raw mut i32 = 0usize as *raw mut i32; \
         const VALUE: i32 = unsafe { POINTER.write(1); 0 };",
    );
    fixture.expect_code("E4031");
}

#[test]
fn an_external_call_is_not_constant() {
    let fixture = Fixture::compile(
        "extern \"c\" { func abs(value: i32) -> i32; } \
         const VALUE: i32 = unsafe { abs(-1) };",
    );
    fixture.expect_code("E4033");
}

#[test]
fn an_external_static_is_not_constant() {
    let fixture = Fixture::compile(
        "extern \"c\" { static errno: i32; } \
         const VALUE: i32 = unsafe { errno };",
    );
    fixture.expect_code("E4034");
}

#[test]
fn with_len_and_as_str_are_not_constant() {
    let fixture = Fixture::compile(
        r#"const P: *raw u8 = c"abc";
           const S: *raw [u8] = unsafe { P.with_len(3) };"#,
    );
    fixture.expect_code("E4032");

    let fixture = Fixture::compile(
        r#"const P: *raw [u8] = &[97u8, 98, 99][..] as *raw [u8];
           const S: *raw str = unsafe { P.as_str() };"#,
    );
    fixture.expect_code("E4032");
}

#[test]
fn a_constant_cannot_read_through_an_integer_pointer() {
    let fixture = Fixture::compile(
        "const POINTER: *raw i32 = 0usize as *raw i32; \
         const VALUE: i32 = unsafe { POINTER.read() };",
    );
    fixture.expect_code("E4032");
}

#[test]
fn dereferencing_a_null_raw_pointer_is_reported() {
    let fixture = Fixture::compile(
        "const POINTER: *raw i32 = 0usize as *raw i32; \
         const VALUE: i32 = unsafe { *POINTER };",
    );
    fixture.expect_code("E4032");
}

#[test]
fn dereferencing_a_misaligned_raw_pointer_is_reported() {
    let fixture = Fixture::compile(
        "static BYTES: [u8; 8] = [0, 1, 2, 3, 4, 5, 6, 7]; \
         const POINTER: *raw i32 = &BYTES[1] as *raw u8 as *raw i32; \
         const VALUE: i32 = unsafe { *POINTER };",
    );
    fixture.expect_code("E4032");
}

#[test]
fn dereferencing_a_pointer_outside_its_allocation_is_reported() {
    let fixture = Fixture::compile(
        "static VALUES: [i32; 2] = [1, 2]; \
         const POINTER: *raw i32 = &VALUES[1] as *raw i32; \
         const PAST_END: *raw i32 = unsafe { POINTER.add(1) }; \
         const VALUE: i32 = unsafe { *PAST_END };",
    );
    fixture.expect_code("E4032");
}

#[test]
fn in_bounds_pointer_arithmetic_stops_at_the_allocation() {
    let fixture = Fixture::compile(
        "static VALUES: [i32; 3] = [1, 2, 3]; \
         const POINTER: *raw i32 = &VALUES[2] as *raw i32; \
         const PAST_END: *raw i32 = unsafe { POINTER.add(1) }; \
         const BEYOND: *raw i32 = unsafe { POINTER.add(2) }; \
         const VALUE: i32 = unsafe { PAST_END.read() };",
    );
    fixture.expect_code("E4032");
}

#[test]
fn a_constant_cannot_inspect_a_raw_pointer_address() {
    let fixture = Fixture::compile(
        "static VALUE: i32 = 7; \
         const POINTER: *raw i32 = &VALUE as *raw i32; \
         const ADDRESS: usize = POINTER.addr();",
    );
    fixture.expect_code("E4030");
}

#[test]
fn a_value_cycle_is_reported_with_both_items() {
    let fixture = Fixture::compile("const A: i32 = B; const B: i32 = A;");
    fixture.expect_code("E4007");
    let messages: Vec<String> = fixture
        .diagnostics()
        .into_iter()
        .map(|diagnostic| diagnostic.message().to_owned())
        .collect();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("A") && message.contains("B")),
        "cycle message lists both items: {messages:?}"
    );
}

#[test]
fn the_step_limit_stops_runaway_evaluation() {
    let limits = EvalLimits {
        steps: 1,
        ..EvalLimits::default()
    };
    let fixture = Fixture::compile_with(
        "static VALUES: [i32; 3] = [1, 2, 3]; static FIRST: i32 = VALUES[0];",
        limits,
    );
    fixture.expect_code("E4008");
}

#[test]
fn the_call_depth_limit_is_enforced() {
    let limits = EvalLimits {
        call_depth: 0,
        ..EvalLimits::default()
    };
    let fixture = Fixture::compile_with("const VALUE: i32 = 1;", limits);
    fixture.expect_code("E4009");
}

#[test]
fn the_allocation_limit_is_enforced() {
    let limits = EvalLimits {
        allocations: 1,
        ..EvalLimits::default()
    };
    let fixture = Fixture::compile_with("static FIRST: i32 = 1; static SECOND: i32 = 2;", limits);
    fixture.expect_code("E4010");
}

#[test]
fn the_value_size_limit_is_enforced() {
    let limits = EvalLimits {
        value_bytes: 0,
        ..EvalLimits::default()
    };
    let fixture = Fixture::compile_with("static VALUE: i32 = 1;", limits);
    fixture.expect_code("E4025");
}

#[test]
fn a_reference_to_a_binding_cannot_escape() {
    let fixture = Fixture::compile("const POINTER: *i32 = { let value = 5; &value };");
    fixture.expect_code("E4013");
}

#[test]
fn out_of_bounds_indexing_is_reported() {
    let fixture = Fixture::compile(
        "const INDEX: usize = 5; \
         const VALUE: i32 = { let values = [1, 2, 3]; values[INDEX] };",
    );
    fixture.expect_code("E4027");
    let messages: Vec<String> = fixture
        .diagnostics()
        .into_iter()
        .map(|diagnostic| diagnostic.message().to_owned())
        .collect();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("index out of bounds")),
        "bounds panic names the failure: {messages:?}"
    );
}

#[test]
fn a_reversed_slice_range_is_reported() {
    let fixture = Fixture::compile(
        "static VALUES: [i32; 3] = [1, 2, 3]; const SLICE: *[i32] = &VALUES[2..1];",
    );
    fixture.expect_code("E4027");
    let messages: Vec<String> = fixture
        .diagnostics()
        .into_iter()
        .map(|diagnostic| diagnostic.message().to_owned())
        .collect();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("slice range")),
        "bounds panic names the failure: {messages:?}"
    );
}

#[test]
fn a_panic_terminator_is_reported() {
    let fixture = Fixture::compile(
        "const INDEX: usize = 9; \
         const VALUE: i32 = { let values = [1]; values[INDEX] };",
    );
    fixture.expect_code("E4027");
}
