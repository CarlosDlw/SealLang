//! End-to-end tests for Seal function pointers.
//!
//! Each program runs the full pipeline and returns zero only when the pointer
//! value, the indirect call, and the internal Seal ABI all behave as declared.

mod support;

use seal_codegen::Emit;
use seal_session::OptLevel;

use support::{TempOutput, compile, run};

fn exits_with(source: &str, tag: &str, status: i32) {
    let output = TempOutput::new(tag);
    compile(source, output.path(), Emit::Executable, OptLevel::O0, true)
        .unwrap_or_else(|error| panic!("the program failed to compile: {error}"));
    assert_eq!(
        run(output.path()).code(),
        Some(status),
        "the program returned the wrong status"
    );
}

#[test]
fn a_function_pointer_calls_its_target() {
    exits_with(
        "\
        func add(a: i32, b: i32) -> i32 { a + b } \
        func main() -> i32 { \
            let apply: func(i32, i32) -> i32 = add; \
            apply(20, 22) \
        }",
        "fn-ptr-basic",
        42,
    );
}

#[test]
fn a_function_pointer_crosses_a_parameter() {
    exits_with(
        "\
        func apply(callback: func(i32) -> i32, value: i32) -> i32 { callback(value) } \
        func triple(value: i32) -> i32 { value * 3 } \
        func main() -> i32 { apply(triple, 14) }",
        "fn-ptr-parameter",
        42,
    );
}

#[test]
fn a_function_returns_a_function_pointer() {
    exits_with(
        "\
        func add_one(value: i32) -> i32 { value + 1 } \
        func chooser() -> func(i32) -> i32 { add_one } \
        func main() -> i32 { let f = chooser(); f(41) }",
        "fn-ptr-return",
        42,
    );
}

#[test]
fn a_function_pointer_lives_in_a_struct_and_a_tuple() {
    exits_with(
        "\
        struct Ops { run: func(i32) -> i32 } \
        func double(value: i32) -> i32 { value * 2 } \
        func main() -> i32 { \
            let ops = Ops { run: double }; \
            let pair = (ops.run, 21); \
            (pair.0)(pair.1) \
        }",
        "fn-ptr-aggregate",
        42,
    );
}

#[test]
fn a_function_pointer_runs_in_a_loop() {
    exits_with(
        "\
        func increment(value: i32) -> i32 { value + 1 } \
        func main() -> i32 { \
            let f = increment; \
            let mut total = 0; \
            let mut index = 0; \
            while index < 40 { total = f(total); index = index + 1; } \
            total + 2 \
        }",
        "fn-ptr-loop",
        42,
    );
}

#[test]
fn a_function_pointer_cast_calls_the_same_target() {
    exits_with(
        "\
        func triple(value: i32) -> i32 { value * 3 } \
        func main() -> i32 { \
            let f = triple as func(i32) -> i32; \
            f(14) \
        }",
        "fn-ptr-cast",
        42,
    );
}

#[test]
fn an_unsafe_function_pointer_calls_inside_unsafe() {
    exits_with(
        "\
        unsafe func add_one(value: i32) -> i32 { value + 1 } \
        func main() -> i32 { \
            let f: unsafe func(i32) -> i32 = add_one; \
            unsafe { f(41) } \
        }",
        "fn-ptr-unsafe",
        42,
    );
}

#[test]
fn a_function_pointer_passes_an_aggregate_by_value() {
    exits_with(
        "\
        struct Pair { left: i32, right: i32 } \
        func sum(pair: Pair) -> i32 { pair.left + pair.right } \
        func main() -> i32 { \
            let f: func(Pair) -> i32 = sum; \
            f(Pair { left: 20, right: 22 }) \
        }",
        "fn-ptr-aggregate-argument",
        42,
    );
}

#[test]
fn a_generic_function_becomes_a_pointer() {
    exits_with(
        "\
        func identity<T>(value: T) -> T { value } \
        func main() -> i32 { \
            let f: func(i32) -> i32 = identity::<i32>; \
            f(42) \
        }",
        "fn-ptr-generic",
        42,
    );
}

#[test]
fn a_function_pointer_call_optimizes_like_a_direct_call() {
    let output = TempOutput::new("fn-ptr-optimized");
    let source = "\
        func add(a: i32, b: i32) -> i32 { a + b } \
        func main() -> i32 { \
            let apply: func(i32, i32) -> i32 = add; \
            apply(20, 22) \
        }";
    compile(source, output.path(), Emit::Executable, OptLevel::O2, false)
        .unwrap_or_else(|error| panic!("the optimized program failed to compile: {error}"));
    assert_eq!(run(output.path()).code(), Some(42));
}
