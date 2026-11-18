//! Code generation for `const`, `static`, and global data.
//!
//! Every scenario runs the real front end, the const evaluator, borrow
//! checking, monomorphization, and code generation. Runtime scenarios check the
//! language-level result at `-O0` and `-O2`; the artifact scenarios inspect the
//! emitted assembly and object because the language cannot print an address or a
//! string's contents.

use std::fs;

use seal_codegen::Emit;
use seal_session::OptLevel;

mod support;

use support::{
    TempOutput, assert_no_thread_locals_or_constructors, compile, contains, is_read_only, run,
    section_of,
};

/// Every global-data form in one program.
///
/// The expected result is 99: 40 + 1 + 2 + 3 + 6 + 7 + 8 + 6 + 1 + 2 + 5 + 9 +
/// 5 + 4.
const GLOBALS: &str = "\
struct Point {
  x: i32,
  y: i32,
}

enum Choice {
  One(i32),
  Two(i32),
}

struct Label {
  tag: i32,
  text: *str,
}

enum Item {
  Label { tag: i32, text: *str },
  Empty,
}

trait Area {
  func area(self: *Self) -> i32;
}

struct Square {
  side: i32,
}

impl Area for Square {
  func area(self: *Square) -> i32 {
    (*self).side * (*self).side
  }
}

static LIMIT: i32 = 40;
static mut TOTAL: i32 = 0;
static POINTS: [i32; 3] = [1, 2, 3];
static ORIGIN: Point = Point { x: 6, y: 7 };
static mut COUNTER: i32 = 5;
const BASE: i32 = 2;
const TABLE: [i32; 3] = [7, 8, 9];
static SECOND: *i32 = &TABLE[1];
static CHOICE: Choice = Choice::Two(6);
static SQUARE: Square = Square { side: 3 };
static SHAPE: *dyn Area = &SQUARE;
static NUMBERS: [i32; 4] = [4, 5, 6, 7];
static VIEW: *[i32] = &NUMBERS[..];
static ITEM: Item = Item::Label { tag: 4, text: \"seal\" };

func next() -> i32 {
  static mut CALLS: i32 = 0;
  unsafe {
    CALLS = CALLS + 1;
  }
  unsafe { CALLS }
}

func main() -> i32 {
  unsafe {
    TOTAL = 1;
  }
  let mut total = LIMIT + unsafe { TOTAL } + BASE;
  total += POINTS[2];
  total += ORIGIN.x + ORIGIN.y;
  total += *SECOND;
  match &CHOICE {
    Choice::One(value) => {
      total += value;
    }
    Choice::Two(value) => {
      total += value;
    }
  }
  total += next() + next();
  total += unsafe { COUNTER };
  total += SHAPE.area();
  total += VIEW[1];
  match &ITEM {
    Item::Label { tag, .. } => {
      total += tag;
    }
    Item::Empty => {}
  }
  total
}
";

#[test]
fn statics_and_constants_produce_their_documented_values() {
    for (opt_level, debug_info) in [(OptLevel::O0, true), (OptLevel::O2, false)] {
        let output = TempOutput::new(&format!("globals-{opt_level:?}"));
        compile(
            GLOBALS,
            output.path(),
            Emit::Executable,
            opt_level,
            debug_info,
        )
        .unwrap_or_else(|error| panic!("the global fixture compiles: {error}"));
        assert_eq!(
            run(output.path()).code(),
            Some(99),
            "the global fixture produced the wrong result at {opt_level:?}"
        );
    }
}

#[test]
fn immutable_statics_are_read_only_and_mutable_ones_are_writable() {
    // Placement is asserted at `-O0`, where every reachable global is emitted
    // under its own symbol. At `-O2` LLVM may fold an internal constant into
    // its uses or rename a global inside a pass, which is correct but makes
    // section inspection meaningless.
    let output = TempOutput::new("globals-sections");
    compile(GLOBALS, output.path(), Emit::Assembly, OptLevel::O0, false)
        .expect("the global fixture emits assembly");
    let assembly = fs::read_to_string(output.path()).expect("the assembly is readable");

    for symbol in [
        "seal.static.LIMIT",
        "seal.static.POINTS",
        "seal.static.ORIGIN",
        "seal.static.SQUARE",
        "seal.static.SHAPE",
        "seal.static.VIEW",
        "seal.static.ITEM",
    ] {
        let section = section_of(&assembly, symbol)
            .unwrap_or_else(|| panic!("{symbol} is not in the assembly:\n{assembly}"));
        assert!(
            is_read_only(&section),
            "{symbol} landed in the writable section {section}"
        );
    }

    let section = section_of(&assembly, "seal.static.COUNTER")
        .expect("a non-zero mutable static has a label");
    assert!(
        !is_read_only(&section),
        "a mutable static landed in {section}"
    );

    // A zero-initialized mutable static is a local common symbol, which the
    // linker places in BSS without storing its zeros.
    for symbol in ["seal.static.TOTAL", "seal.static.next.CALLS"] {
        assert!(
            assembly.contains(&format!(".comm\t{symbol},")),
            "{symbol} is not a zero-initialized common symbol:\n{assembly}"
        );
    }

    assert_no_thread_locals_or_constructors(&assembly);
}

#[test]
fn zero_sized_statics_receive_distinct_one_byte_storage() {
    // At `-O2` an unobserved zero-sized static is removable, so the backing
    // storage is checked at `-O0`, where every reachable global is emitted.
    let output = TempOutput::new("globals-zero-sized");
    compile(
        "\
static UNIT_A: () = ();
static UNIT_B: () = ();

func main() -> i32 {
  let a = UNIT_A;
  let b = UNIT_B;
  a;
  b;
  0
}
",
        output.path(),
        Emit::Assembly,
        OptLevel::O0,
        false,
    )
    .expect("zero-sized statics compile");
    let assembly = fs::read_to_string(output.path()).expect("the assembly is readable");

    for symbol in ["seal.static.UNIT_A", "seal.static.UNIT_B"] {
        assert!(
            assembly.contains(&format!(".size\t{symbol}, 1")),
            "{symbol} does not occupy exactly one byte:\n{assembly}"
        );
    }
    assert_ne!(
        section_of(&assembly, "seal.static.UNIT_A"),
        None,
        "the first zero-sized static has no label"
    );
    let first = assembly
        .find("seal.static.UNIT_A:")
        .expect("the first zero-sized static has a label");
    let second = assembly
        .find("seal.static.UNIT_B:")
        .expect("the second zero-sized static has a label");
    assert_ne!(first, second, "the statics must not share one address");
}

#[test]
fn a_static_trait_object_is_relocated_without_a_constructor() {
    for opt_level in [OptLevel::O0, OptLevel::O2] {
        let output = TempOutput::new(&format!("globals-object-{opt_level:?}"));
        compile(GLOBALS, output.path(), Emit::Object, opt_level, false)
            .expect("the global fixture emits an object");
        let object = fs::read(output.path()).expect("the object is readable");

        // At `-O2` the constants may be folded into their uses, so only `-O0`
        // keeps every symbol. The absence of constructors and thread-local
        // storage must hold at every level.
        if opt_level == OptLevel::O0 {
            for symbol in [
                "seal.static.SHAPE",
                "seal.static.SQUARE",
                "seal.static.LIMIT",
            ] {
                assert!(
                    contains(&object, symbol.as_bytes()),
                    "the object does not name {symbol}"
                );
            }
        }
        assert!(
            !contains(&object, b"llvm.global_ctors"),
            "the object contains a static constructor at {opt_level:?}"
        );
        assert!(
            !contains(&object, b".tdata") && !contains(&object, b".tbss"),
            "the object contains thread-local storage at {opt_level:?}"
        );
    }
}
