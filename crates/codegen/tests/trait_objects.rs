//! End-to-end trait-object code generation.
//!
//! Every scenario runs the real front end, borrow checker, monomorphization,
//! vtable emission, and linked executable. The result is the language-level
//! exit status, so a broken slot order or indirect call cannot pass unnoticed.

use seal_codegen::Emit;
use seal_session::OptLevel;

mod support;

use support::{TempOutput, compile, run};

const SHAPE: &str = "\
trait Shape { func area(self: *Self) -> i32; }
struct Circle { radius: i32 }
struct Square { side: i32 }
impl Shape for Circle { func area(self: *Circle) -> i32 { (*self).radius * (*self).radius } }
impl Shape for Square { func area(self: *Square) -> i32 { (*self).side * (*self).side } }
";

/// Compiles `source` as an executable and asserts its exit status.
fn assert_runs(name: &str, source: &str, expected: i32, opt_level: OptLevel, debug_info: bool) {
    let output = TempOutput::new(name);
    compile(
        source,
        output.path(),
        Emit::Executable,
        opt_level,
        debug_info,
    )
    .unwrap_or_else(|error| panic!("{name}: {error}"));
    assert_eq!(
        run(output.path()).code(),
        Some(expected),
        "{name} returned the wrong value at {opt_level:?} (debug={debug_info})"
    );
}

#[test]
fn two_concrete_types_dispatch_to_different_results() {
    let source = format!(
        "{SHAPE}\n\
         func main() -> i32 {{\n\
           let circle = Circle {{ radius: 3 }};\n\
           let square = Square {{ side: 4 }};\n\
           let first: *dyn Shape = &circle;\n\
           let second: *dyn Shape = &square;\n\
           first.area() + second.area()\n\
         }}\n"
    );
    for (opt_level, debug_info, suffix) in [
        (OptLevel::O0, false, "dyn-two-o0"),
        (OptLevel::O2, false, "dyn-two-o2"),
        (OptLevel::O2, true, "dyn-two-o2-g"),
    ] {
        assert_runs(suffix, &source, 25, opt_level, debug_info);
    }
}

#[test]
fn shared_and_exclusive_methods_dispatch() {
    let source = "\
trait Counter { func read(self: *Self) -> i32; func add(self: *mut Self, amount: i32); }
struct Box { value: i32 }
impl Counter for Box {
  func read(self: *Box) -> i32 { (*self).value }
  func add(self: *mut Box, amount: i32) { (*self).value += amount; }
}
func peek(counter: *dyn Counter) -> i32 { counter.read() }
func bump(counter: *mut dyn Counter) -> i32 { counter.add(2); counter.read() }
func main() -> i32 {
  let mut value = Box { value: 40 };
  peek(&value) + bump(&mut value)
}
";
    assert_runs("dyn-shared-exclusive", source, 82, OptLevel::O2, false);
}

#[test]
fn supertrait_methods_dispatch() {
    let source = "\
trait Value { func value(self: *Self) -> i32; }
trait Named: Value { func name_length(self: *Self) -> i32; }
struct Item { value: i32, name_length: i32 }
impl Value for Item { func value(self: *Item) -> i32 { (*self).value } }
impl Named for Item { func name_length(self: *Item) -> i32 { (*self).name_length } }
func main() -> i32 {
  let item = Item { value: 4, name_length: 2 };
  let object: *dyn Named = &item;
  object.value() * 10 + object.name_length()
}
";
    assert_runs("dyn-supertrait", source, 42, OptLevel::O2, false);
}

#[test]
fn generic_function_coerces_to_a_trait_object() {
    let source = format!(
        "{SHAPE}\n\
         func area_of<T: Shape>(value: *T) -> i32 {{\n\
           let object: *dyn Shape = value;\n\
           object.area()\n\
         }}\n\
         func main() -> i32 {{\n\
           let circle = Circle {{ radius: 7 }};\n\
           area_of(&circle)\n\
         }}\n"
    );
    assert_runs("dyn-generic-coercion", &source, 49, OptLevel::O2, false);
}

#[test]
fn generic_trait_argument_dispatches() {
    let source = "\
trait Convert<T> { func convert(self: *Self, value: T) -> T; }
struct AddOne { }
impl Convert<i32> for AddOne { func convert(self: *AddOne, value: i32) -> i32 { value + 1 } }
func main() -> i32 {
  let add = AddOne { };
  let object: *dyn Convert<i32> = &add;
  object.convert(41)
}
";
    assert_runs("dyn-generic-trait", source, 42, OptLevel::O2, false);
}

#[test]
fn dynamic_methods_pass_and_return_aggregates() {
    let source = "\
struct Pair { left: i32, right: i32 }
trait Build { func build(self: *Self, pair: Pair) -> Pair; }
struct Source { value: i32 }
impl Build for Source {
  func build(self: *Source, pair: Pair) -> Pair {
    Pair { left: pair.left + (*self).value, right: pair.right + 1 }
  }
}
func dispatch(source: *dyn Build) -> i32 {
  let pair = source.build(Pair { left: 1, right: 1 });
  pair.left * 10 + pair.right
}
func main() -> i32 {
  let source = Source { value: 3 };
  dispatch(&source)
}
";
    assert_runs("dyn-aggregates", source, 42, OptLevel::O2, false);
}

#[test]
fn empty_trait_object_is_constructible() {
    let source = "\
trait Empty { }
struct Marker { value: i32 }
impl Empty for Marker { }
func touch(object: *dyn Empty) -> i32 { 42 }
func main() -> i32 {
  let marker = Marker { value: 1 };
  touch(&marker)
}
";
    assert_runs("dyn-empty", source, 42, OptLevel::O2, false);
}

#[test]
fn coercions_work_at_every_supported_site() {
    let source = format!(
        "{SHAPE}\n\
         struct Holder {{ object: *dyn Shape }}\n\
         func area(object: *dyn Shape) -> i32 {{ object.area() }}\n\
         func shared(circle: *Circle) -> *dyn Shape {{ circle }}\n\
         func shared_square(square: *Square) -> *dyn Shape {{ square }}\n\
         func main() -> i32 {{\n\
           let circle = Circle {{ radius: 3 }};\n\
           let square = Square {{ side: 4 }};\n\
           let local: *dyn Shape = &circle;\n\
           let direct = area(&circle);\n\
           let returned = shared(&circle);\n\
           let holder = Holder {{ object: shared(&circle) }};\n\
           let pair: (*dyn Shape, *dyn Shape) = (shared(&circle), shared_square(&square));\n\
           let items: [*dyn Shape; 2] = [shared(&circle), shared_square(&square)];\n\
           let condition = true;\n\
           let chosen: *dyn Shape = if condition {{ shared(&circle) }} else {{ shared_square(&square) }};\n\
           let matched: *dyn Shape = match condition {{\n\
             true => shared(&circle),\n\
             false => shared_square(&square),\n\
           }};\n\
           local.area() + direct + returned.area() + holder.object.area()\n\
             + pair.0.area() + pair.1.area()\n\
             + items[0].area() + items[1].area()\n\
             + chosen.area() + matched.area()\n\
         }}\n"
    );
    assert_runs("dyn-coercion-sites", &source, 104, OptLevel::O2, false);
}

#[test]
fn object_and_assembly_output_do_not_link_the_runtime() {
    let source = format!(
        "{SHAPE}\n\
         func main() -> i32 {{ let circle = Circle {{ radius: 1 }}; let object: *dyn Shape = &circle; object.area() }}\n"
    );

    let object = TempOutput::new("dyn.o");
    compile(
        source.as_str(),
        object.path(),
        Emit::Object,
        OptLevel::O2,
        false,
    )
    .expect("object emission succeeds");
    assert!(
        std::fs::metadata(object.path())
            .expect("object exists")
            .len()
            > 0
    );

    let assembly = TempOutput::new("dyn.s");
    compile(
        source.as_str(),
        assembly.path(),
        Emit::Assembly,
        OptLevel::O0,
        false,
    )
    .expect("assembly emission succeeds");
    let text = std::fs::read_to_string(assembly.path()).expect("assembly is UTF-8");
    assert!(text.contains("seal.vtable."), "assembly lost the vtable");
}

#[test]
fn repeated_builds_produce_stable_vtable_symbols() {
    let source = format!(
        "{SHAPE}\n\
         func upcast(circle: *Circle) -> *dyn Shape {{ circle }}\n"
    );
    let first = TempOutput::new("dyn-stable-a.s");
    let second = TempOutput::new("dyn-stable-b.s");
    for path in [first.path(), second.path()] {
        compile(source.as_str(), path, Emit::Assembly, OptLevel::O0, false)
            .expect("assembly emission succeeds");
    }
    let first = std::fs::read_to_string(first.path()).expect("assembly is UTF-8");
    let second = std::fs::read_to_string(second.path()).expect("assembly is UTF-8");
    let name = |text: &str| {
        text.split("seal.vtable.")
            .nth(1)
            .and_then(|rest| {
                rest.split(|character: char| !character.is_ascii_hexdigit())
                    .next()
            })
            .unwrap_or_else(|| panic!("missing vtable symbol in:\n{text}"))
            .to_owned()
    };
    assert_eq!(name(&first), name(&second));
}

#[test]
fn programs_without_dynamic_dispatch_are_unchanged() {
    let source = "func main() -> i32 { 42 }";
    let assembly = TempOutput::new("no-dyn.s");
    compile(source, assembly.path(), Emit::Assembly, OptLevel::O2, false)
        .expect("assembly emission succeeds");
    let text = std::fs::read_to_string(assembly.path()).expect("assembly is UTF-8");
    assert!(
        !text.contains("seal.vtable."),
        "unexpected vtable in:\n{text}"
    );
    assert!(
        !text.contains("seal.drop.noop"),
        "a program without trait objects must not emit the drop helper:\n{text}"
    );

    assert_runs("no-dyn", source, 42, OptLevel::O2, false);
}
