//! End-to-end tests for first-class functions and closures.

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
fn a_closure_reads_its_capture() {
    exits_with(
        "func main() -> i32 { let value = 40; let add = || value + 2; add() }",
        "closure-fn",
        42,
    );
}

#[test]
fn a_fn_mut_closure_mutates_state_between_calls() {
    exits_with(
        "func main() -> i32 { \
             let mut total = 0; \
             let mut add = |value: i32| { total += value; }; \
             add(2); add(3); total \
         }",
        "closure-fn-mut",
        5,
    );
}

#[test]
fn a_fn_once_closure_consumes_its_capture() {
    exits_with(
        "struct Token { value: i32 } \
         func main() -> i32 { \
             let token = Token { value: 21 }; \
             let take = move || token; \
             let consumed = take(); \
             consumed.value * 2 \
         }",
        "closure-fn-once",
        42,
    );
}

#[test]
fn a_move_capture_is_a_snapshot() {
    exits_with(
        "func main() -> i32 { \
             let mut value = 10; \
             let read = move || value; \
             value = 32; \
             read() \
         }",
        "closure-move-snapshot",
        10,
    );
}

#[test]
fn a_capture_free_closure_calls_through_a_function_pointer() {
    exits_with(
        "func main() -> i32 { let add: func(i32) -> i32 = |value| value + 1; add(41) }",
        "closure-fn-pointer",
        42,
    );
}

#[test]
fn the_canonical_first_class_function_example_runs() {
    exits_with(
        "func add_one(a: i32) -> i32 { a + 1 } \
         func apply(f: func(i32) -> i32, value: i32) -> i32 { f(value) } \
         func main() -> i32 { \
             let f: func(i32) -> i32 = add_one; \
             let g = |x: i32| x * 2; \
             let h = move |x: i32| x + 1; \
             apply(f, 20) + apply(g, 3) + apply(h, 4) \
         }",
        "first-class-canonical",
        32,
    );
}

#[test]
fn the_canonical_twice_bound_runs() {
    exits_with(
        "func twice<F: Fn() -> i32>(f: F) -> i32 { f() + f() } \
         func main() -> i32 { let base = 10; twice(move || base + 1) }",
        "closure-twice",
        22,
    );
}

#[test]
fn the_canonical_each_bound_mutates_the_closure() {
    exits_with(
        "func each<F: FnMut(i32)>(f: F, n: i32) { \
             let mut f = f; \
             let mut index = 0; \
             while index < n { f(index); index += 1; } \
         } \
         func main() -> i32 { \
             let mut total = 0; \
             each(|value| { total += value; }, 4); \
             total \
         }",
        "closure-each",
        6,
    );
}

#[test]
fn the_canonical_consume_bound_runs() {
    exits_with(
        "func consume<F: FnOnce() -> i32>(f: F) -> i32 { f() } \
         func main() -> i32 { let base = 41; consume(move || base + 1) }",
        "closure-consume",
        42,
    );
}

#[test]
fn a_nested_closure_reads_the_outer_environment() {
    exits_with(
        "func main() -> i32 { \
             let value = 40; \
             let outer = || { let inner = || value + 2; inner() }; \
             outer() \
         }",
        "closure-nested",
        42,
    );
}

#[test]
fn a_generic_apply_accepts_a_capturing_closure() {
    exits_with(
        "func apply<F: Fn(i32) -> i32>(f: F, value: i32) -> i32 { f(value) } \
         func main() -> i32 { let base = 40; apply(|value| value + base, 2) }",
        "closure-generic-apply",
        42,
    );
}

#[test]
fn function_pointers_return_and_live_in_aggregates() {
    exits_with(
        "struct Ops { run: func(i32) -> i32 } \
         func double(value: i32) -> i32 { value * 2 } \
         func chooser() -> func(i32) -> i32 { double } \
         func main() -> i32 { \
             let chosen = chooser(); \
             let ops = Ops { run: chosen }; \
             let pair = (ops.run, 10); \
             let array = [pair.0, chosen]; \
             (pair.0)(pair.1) + array[1](1) \
         }",
        "fn-pointer-aggregates",
        22,
    );
}
