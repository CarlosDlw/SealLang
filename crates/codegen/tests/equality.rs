//! Equality lowering tests.
//!
//! Every program returns 42 only when all of its comparisons agree with the
//! language rules, so a failure names the first comparison that disagreed.

use seal_codegen::Emit;
use seal_session::OptLevel;

mod support;

use support::{TempOutput, compile, run};

/// Runs `source` at both optimization levels and asserts its exit status.
fn assert_program(source: &str, expected: i32) {
    for level in [OptLevel::O0, OptLevel::O2] {
        let output = TempOutput::new("equality");
        compile(source, output.path(), Emit::Executable, level, false)
            .expect("the equality program compiles");
        assert_eq!(
            run(output.path()).code(),
            Some(expected),
            "unexpected status at optimization level {}",
            level.as_u8()
        );
    }
}

#[test]
fn string_references_compare_their_bytes() {
    assert_program(
        "\
func main() -> i32 {
  let a: *str = \"hello\";
  let b: *str = \"hello\";
  let c: *str = \"hellp\";
  let d: *str = \"hell\";
  let e: *str = \"hellos\";
  let empty: *str = \"\";
  let other: *str = \"x\";
  if a != b { return 1; }
  if !(a == b) { return 2; }
  if a == c { return 3; }
  if a == d { return 4; }
  if a == e { return 5; }
  if empty == other { return 6; }
  if empty != empty { return 7; }
  if \"seal\" != \"seal\" { return 8; }
  42
}
",
        42,
    );
}

#[test]
fn slice_references_compare_length_and_elements() {
    assert_program(
        "\
struct Point { x: i32, y: i32 }
func main() -> i32 {
  let values: [i32; 3] = [1, 2, 3];
  let same: [i32; 3] = [1, 2, 3];
  let other: [i32; 3] = [1, 2, 4];
  let all: *[i32] = &values[..];
  let all_same: *[i32] = &same[..];
  let all_other: *[i32] = &other[..];
  let prefix: *[i32] = &values[0..2];
  let suffix: *[i32] = &values[1..3];
  let empty: *[i32] = &values[0..0];
  if all != all_same { return 1; }
  if !(all == all_same) { return 2; }
  if all == all_other { return 3; }
  if all == prefix { return 4; }
  if all == suffix { return 5; }
  if empty != empty { return 6; }
  if empty == all { return 7; }

  let points: [Point; 2] = [Point { x: 1, y: 2 }, Point { x: 3, y: 4 }];
  let points_same: [Point; 2] = [Point { x: 1, y: 2 }, Point { x: 3, y: 4 }];
  let points_other: [Point; 2] = [Point { x: 1, y: 2 }, Point { x: 3, y: 5 }];
  let left: *[Point] = &points[..];
  let right: *[Point] = &points_same[..];
  let mismatch: *[Point] = &points_other[..];
  if left != right { return 8; }
  if left == mismatch { return 9; }

  let mut mutable_values: [i32; 2] = [5, 6];
  let mut mutable_same: [i32; 2] = [5, 6];
  let mut mutable_other: [i32; 2] = [5, 7];
  let left_mut: *mut [i32] = &mut mutable_values[..];
  let right_mut: *mut [i32] = &mut mutable_same[..];
  let other_mut: *mut [i32] = &mut mutable_other[..];
  if left_mut != right_mut { return 10; }
  if left_mut == other_mut { return 11; }
  42
}
",
        42,
    );
}

#[test]
fn structs_compare_field_by_field() {
    assert_program(
        "\
struct Point { x: i32, y: i32 }
struct Named { name: *str, id: i32 }
func main() -> i32 {
  let a: Point = Point { x: 1, y: 2 };
  let same: Point = Point { x: 1, y: 2 };
  let different_x: Point = Point { x: 2, y: 2 };
  let different_y: Point = Point { x: 1, y: 3 };
  if a != same { return 1; }
  if a == different_x { return 2; }
  if a == different_y { return 3; }

  // Equality reads its operands, so both stay usable afterwards.
  if a == different_x { return 4; }
  if a != same { return 5; }

  let n: Named = Named { name: \"ann\", id: 3 };
  let n_same: Named = Named { name: \"ann\", id: 3 };
  let n_name: Named = Named { name: \"ana\", id: 3 };
  let n_id: Named = Named { name: \"ann\", id: 4 };
  if n != n_same { return 6; }
  if n == n_name { return 7; }
  if n == n_id { return 8; }

  // A comparison reads its operands, so a pointee and a static are values it
  // can compare without moving them.
  let left: *Point = &a;
  let right: *Point = &same;
  let other: *Point = &different_x;
  if *left != *right { return 9; }
  if *left == *other { return 10; }
  let origin: Point = Point { x: 0, y: 0 };
  if a == ORIGIN { return 11; }
  if ORIGIN != ORIGIN { return 12; }
  if origin != ORIGIN { return 13; }
  42
}

static ORIGIN: Point = Point { x: 0, y: 0 };
",
        42,
    );
}

#[test]
fn enums_compare_the_active_variant() {
    assert_program(
        "\
enum Shape {
  Dot,
  Line(i32, i32),
  Box { side: i32 },
}
func main() -> i32 {
  let dot: Shape = Shape::Dot;
  let dot_same: Shape = Shape::Dot;
  let line: Shape = Shape::Line(1, 2);
  let line_same: Shape = Shape::Line(1, 2);
  let line_other: Shape = Shape::Line(1, 3);
  let boxed: Shape = Shape::Box { side: 1 };
  let boxed_same: Shape = Shape::Box { side: 1 };
  let boxed_other: Shape = Shape::Box { side: 2 };
  if dot != dot_same { return 1; }
  if dot == line { return 2; }
  if dot == boxed { return 3; }
  if line != line_same { return 4; }
  if line == line_other { return 5; }
  if line == boxed { return 6; }
  if boxed != boxed_same { return 7; }
  if boxed == boxed_other { return 8; }
  if line == line_same { } else { return 9; }
  42
}
",
        42,
    );
}

#[test]
fn tuples_and_arrays_compare_element_by_element() {
    assert_program(
        "\
func main() -> i32 {
  let pair: (i32, bool) = (7, true);
  let pair_same: (i32, bool) = (7, true);
  let pair_flag: (i32, bool) = (7, false);
  let pair_number: (i32, bool) = (8, true);
  if pair != pair_same { return 1; }
  if pair == pair_flag { return 2; }
  if pair == pair_number { return 3; }

  let nested: ((i32, char), [bool; 2]) = ((1, 'a'), [true, false]);
  let nested_same: ((i32, char), [bool; 2]) = ((1, 'a'), [true, false]);
  let nested_char: ((i32, char), [bool; 2]) = ((1, 'b'), [true, false]);
  let nested_array: ((i32, char), [bool; 2]) = ((1, 'a'), [true, true]);
  if nested != nested_same { return 4; }
  if nested == nested_char { return 5; }
  if nested == nested_array { return 6; }

  let empty: [i32; 0] = [];
  let empty_same: [i32; 0] = [];
  if empty != empty_same { return 7; }
  let unit_a: () = ();
  let unit_b: () = ();
  if unit_a != unit_b { return 8; }
  42
}
",
        42,
    );
}

#[test]
fn nested_aggregates_compare_recursively() {
    assert_program(
        "\
struct Point { x: i32, y: i32 }
enum Shape { Dot, Circle(i32) }
struct Drawing { name: *str, points: *[Point], shape: Shape }
func main() -> i32 {
  let points: [Point; 2] = [Point { x: 1, y: 2 }, Point { x: 3, y: 4 }];
  let same: [Point; 2] = [Point { x: 1, y: 2 }, Point { x: 3, y: 4 }];
  let a: Drawing = Drawing { name: \"dot\", points: &points[..], shape: Shape::Dot };
  let b: Drawing = Drawing { name: \"dot\", points: &same[..], shape: Shape::Dot };
  let c: Drawing = Drawing { name: \"dot\", points: &points[..], shape: Shape::Circle(1) };
  let d: Drawing = Drawing { name: \"dots\", points: &points[..], shape: Shape::Dot };
  if a != b { return 1; }
  if a == c { return 2; }
  if a == d { return 3; }

  let table: [(i32, *str); 2] = [(1, \"one\"), (2, \"two\")];
  let table_same: [(i32, *str); 2] = [(1, \"one\"), (2, \"two\")];
  let table_other: [(i32, *str); 2] = [(1, \"one\"), (2, \"three\")];
  if table != table_same { return 4; }
  if table == table_other { return 5; }
  42
}
",
        42,
    );
}

#[test]
fn floating_point_equality_follows_ieee() {
    assert_program(
        "\
struct Real { value: f64 }
func main() -> i32 {
  let zero: f64 = 0.0;
  let negative_zero: f64 = -0.0;
  let nan: f64 = zero / zero;
  if negative_zero != zero { return 1; }
  if nan == nan { return 2; }
  if !(nan != nan) { return 3; }

  let a: Real = Real { value: nan };
  let b: Real = Real { value: nan };
  let c: Real = Real { value: zero };
  let d: Real = Real { value: negative_zero };
  if a == b { return 4; }
  if c != d { return 5; }

  let values: [f64; 2] = [1.5, nan];
  let values_same: [f64; 2] = [1.5, nan];
  if values == values_same { return 6; }
  42
}
",
        42,
    );
}

#[test]
fn raw_pointers_compare_data_addresses_only() {
    assert_program(
        "\
func main() -> i32 {
  let first = 1;
  let second = 2;
  let p = &first as *raw i32;
  let q = &first as *raw i32;
  let r = &second as *raw i32;
  if p != q { return 1; }
  if !(p == q) { return 2; }
  if p == r { return 3; }

  let values = [1, 2, 3];
  let whole: *raw [i32] = &values[..] as *raw [i32];
  let prefix: *raw [i32] = &values[0..1] as *raw [i32];
  let tail: *raw [i32] = &values[1..2] as *raw [i32];
  if whole != prefix { return 4; }
  if whole == tail { return 5; }

  let null: *raw i32 = 0usize as *raw i32;
  let other_null: *raw i32 = 0usize as *raw i32;
  if null != other_null { return 6; }
  42
}
",
        42,
    );
}
