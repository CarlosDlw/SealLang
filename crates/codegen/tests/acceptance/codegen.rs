//! Acceptance scenarios for monomorphization, ABI, and lowering.

use seal_codegen::Emit;
use seal_session::OptLevel;

use crate::assert_runs;
use crate::support::{TempOutput, compile};

#[test]
fn three_instances_of_one_function_produce_distinct_results() {
    assert_runs(
        "three-instances",
        r#"
trait Value { func value(self: *Self) -> i32; }

impl Value for i32 { func value(self: *i32) -> i32 { *self } }

impl Value for bool { func value(self: *bool) -> i32 { if *self { 1 } else { 0 } } }

struct Pair<A, B> { first: A, second: B }

impl<A: Value, B: Value> Value for Pair<A, B> {
  func value(self: *Pair<A, B>) -> i32 {
    (*self).first.value() + (*self).second.value()
  }
}

func read<T: Value>(value: *T) -> i32 { value.value() }

func main() -> i32 {
  let number = 40;
  let flag = true;
  let pair = Pair { first: 0, second: true };
  read(&number) + read(&flag) + read(&pair)
}
"#,
        42,
    );
}

#[test]
fn one_struct_instantiated_with_two_layouts_keeps_its_fields() {
    assert_runs(
        "struct-layouts",
        r#"
struct Pair<T, U> { first: T, second: U }

func main() -> i32 {
  let numbers = Pair { first: 40, second: true };
  let flags = Pair { first: true, second: 2 };
  numbers.first + flags.second
}
"#,
        42,
    );
}

#[test]
fn one_enum_instantiated_with_two_payload_layouts_keeps_its_payloads() {
    assert_runs(
        "enum-layouts",
        r#"
enum Option<T> { Some(T), None }

struct Pair<T> { first: T, second: T }

func main() -> i32 {
  let number = Option::Some(40);
  let pair = Option::Some(Pair { first: 1, second: 1 });
  let a = match number { Option::Some(value) => value, Option::None => 0 };
  let b = match pair {
    Option::Some(value) => value.first + value.second,
    Option::None => 0,
  };
  a + b
}
"#,
        42,
    );
}

#[test]
fn recursive_generic_functions_with_a_fixed_argument_terminate() {
    assert_runs(
        "recursive-generic",
        r#"
func countdown<T: Copy>(remaining: i32, value: T) -> T {
  if remaining == 0 { value } else { countdown(remaining - 1, value) }
}

func main() -> i32 {
  countdown(4, 42)
}
"#,
        42,
    );
}

#[test]
fn mutually_recursive_generic_functions_with_fixed_arguments_terminate() {
    assert_runs(
        "mutually-recursive-generics",
        r#"
func even<T: Copy>(remaining: i32, value: T) -> T {
  if remaining == 0 { value } else { odd(remaining - 1, value) }
}

func odd<T: Copy>(remaining: i32, value: T) -> T {
  if remaining == 0 { value } else { even(remaining - 1, value) }
}

func main() -> i32 {
  even(4, 42)
}
"#,
        42,
    );
}

#[test]
fn a_trait_call_emits_a_direct_call_to_the_selected_implementation() {
    let source = r#"
trait Value { func value(self: *Self) -> i32; }

impl Value for i32 { func value(self: *i32) -> i32 { *self } }

func read<T: Value>(value: *T) -> i32 { value.value() }

func main() -> i32 {
  let number = 42;
  read(&number)
}
"#;
    let output = TempOutput::new("direct-trait-call.s");
    compile(source, output.path(), Emit::Assembly, OptLevel::O0, false)
        .expect("the program compiles to assembly");
    let bytes = std::fs::read(output.path()).expect("assembly is readable");
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("seal.Self.value"),
        "the selected implementation is missing from the assembly"
    );
    assert!(
        text.contains("callq\tseal.Self.value"),
        "the trait call is not direct"
    );
    assert!(
        !text.contains("callq\t*"),
        "the trait call must not go through a function pointer"
    );
}

#[test]
fn generic_code_preserves_the_fat_pointer_abi() {
    assert_runs(
        "fat-pointers",
        r#"
func identity<T>(value: T) -> T { value }

func text_score(value: *str) -> i32 {
  let copy = identity(value);
  40
}

func slice_len<T>(values: *[T]) -> usize { values.len() }

func main() -> i32 {
  let values = [1, 2];
  let slice = &values[0..2];
  if slice_len(identity(slice)) == 2 && text_score(identity("seal")) == 40 { 42 } else { 0 }
}
"#,
        42,
    );
}
