//! End-to-end regression tests for the diagnostics users see on the command
//! line.
//!
//! Every case is a short program that must be rejected with a stable code, a
//! message fragment, and exit status 1. A diagnostic that regresses to an
//! internal compiler error, changes its code, or silently accepts the program
//! fails here instead of reaching a user.

mod support;

use support::{SourceFile, reject};

#[test]
fn moving_an_array_element_reports_e5015() {
    reject(
        "move-array-element",
        r#"struct Token {
  value: i32,
}

func main() -> i32 {
  let items = [Token { value: 21 }, Token { value: 7 }];
  let first = items[0];
  first.value + items[1].value
}
"#,
        "E5015",
        "cannot move an element out of an array",
    );
}

#[test]
fn assigning_while_borrowed_reports_e5001() {
    reject(
        "assign-borrowed",
        r#"func main() -> i32 {
  let mut x = 1;
  let r = &x;
  x = 2;
  *r
}
"#,
        "E5001",
        "because it is borrowed",
    );
}

#[test]
fn moving_while_borrowed_reports_e5002() {
    reject(
        "move-borrowed",
        r#"struct Point {
  x: i32,
  y: i32,
}

func main() -> i32 {
  let p = Point { x: 1, y: 2 };
  let r = &p;
  let q = p;
  (*r).x + q.y
}
"#,
        "E5002",
        "because it is borrowed",
    );
}

#[test]
fn returning_a_local_reference_reports_e5004() {
    reject(
        "return-local",
        r#"func pick() -> *i32 {
  let x = 41;
  &x
}

func main() -> i32 { 0 }
"#,
        "E5004",
        "local of this function",
    );
}

#[test]
fn a_use_of_a_rejected_borrow_does_not_cascade() {
    let file = SourceFile::new(
        "no-cascade",
        r#"func main() -> i32 {
  let mut x = 1;
  let a = &mut x;
  let b = &x;
  *a + *b
}
"#,
    );
    let output = file.check();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error[E5008]"), "expected E5008: {stderr}");
    assert!(
        !stderr.contains("error[E5007]"),
        "a rejected borrow must not cascade: {stderr}"
    );
}

#[test]
fn a_returned_local_reference_points_at_the_returned_expression() {
    let file = SourceFile::new(
        "return-label",
        r#"func pick() -> *i32 {
  let value = 41;
  &value
}

func main() -> i32 { 0 }
"#,
    );
    let output = file.check();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("error[E5004]") && stderr.contains("returned here"),
        "expected the returned expression to be labeled: {stderr}"
    );
}

#[test]
fn returning_a_reference_through_a_local_borrow_reports_e5004() {
    reject(
        "return-through-value",
        r#"func identity(p: *i32) -> *i32 {
  p
}

func main() -> *i32 {
  let x = 41;
  &*identity(&x)
}
"#,
        "E5004",
        "local of this function",
    );
}

#[test]
fn reading_a_moved_value_reports_e5005() {
    reject(
        "use-moved",
        r#"struct Point {
  x: i32,
  y: i32,
}

func main() -> i32 {
  let a = Point { x: 1, y: 2 };
  let b = a;
  a.x + b.y
}
"#,
        "E5005",
        "because `a` was moved",
    );
}

#[test]
fn borrowing_a_partially_moved_struct_reports_e5006() {
    reject(
        "partial-borrow",
        r#"struct Point {
  x: i32,
  y: i32,
}

struct Segment {
  start: Point,
  end: Point,
}

func main() -> i32 {
  let s = Segment { start: Point { x: 1, y: 2 }, end: Point { x: 3, y: 4 } };
  let start = s.start;
  let r = &s;
  (*r).end.x + start.x
}
"#,
        "E5006",
        "partially moved",
    );
}

#[test]
fn assigning_an_immutable_binding_reports_e3004() {
    reject(
        "immutable",
        r#"func main() -> i32 {
  let x = 1;
  x = 2;
  x
}
"#,
        "E3004",
        "not declared as mutable",
    );
}

#[test]
fn reading_an_uninitialized_binding_reports_e3011() {
    reject(
        "uninitialized",
        r#"func main() -> i32 {
  let x: i32;
  x
}
"#,
        "E3011",
        "possibly uninitialized",
    );
}

#[test]
fn dereferencing_a_non_reference_reports_e3012() {
    reject(
        "deref-non-reference",
        r#"func main() -> i32 {
  let x = 1;
  *x
}
"#,
        "E3012",
        "cannot dereference",
    );
}

#[test]
fn borrowing_a_computed_value_reports_e3013() {
    reject(
        "borrow-value",
        r#"func main() -> i32 {
  let r = &(1 + 2);
  *r
}
"#,
        "E3013",
        "cannot borrow this expression",
    );
}

#[test]
fn mutably_borrowing_an_immutable_binding_reports_e3017() {
    reject(
        "mutable-borrow-immutable",
        r#"func main() {
  let x = 1;
  let r = &mut x;
}
"#,
        "E3017",
        "not declared as mutable",
    );
}

#[test]
fn moving_out_of_a_reference_reports_e3018() {
    reject(
        "move-out-of-reference",
        r#"struct Point {
  x: i32,
}

func main() -> Point {
  let mut p = Point { x: 1 };
  let r = &mut p;
  *r
}
"#,
        "E3018",
        "cannot move out of a reference",
    );
}

#[test]
fn reading_while_exclusively_borrowed_reports_e5007() {
    reject(
        "read-exclusive-borrow",
        r#"func main() -> i32 {
  let mut x = 1;
  let r = &mut x;
  let y = x;
  *r + y
}
"#,
        "E5007",
        "exclusively borrowed",
    );
}

#[test]
fn borrowing_exclusively_twice_reports_e5008() {
    reject(
        "double-exclusive-borrow",
        r#"func main() -> i32 {
  let mut x = 1;
  let a = &mut x;
  let b = &mut x;
  *a + *b
}
"#,
        "E5008",
        "already borrowed",
    );
}

#[test]
fn a_borrow_conflict_labels_the_last_use() {
    reject(
        "conflict-last-use",
        r#"func main() -> i32 {
  let mut x = 1;
  let r = &mut x;
  x = 2;
  *r
}
"#,
        "E5008",
        "borrow is used here",
    );
}

#[test]
fn exclusive_references_are_accepted() {
    let file = SourceFile::new(
        "exclusive-accepted",
        r#"func bump(p: *mut i32) {
  *p = *p + 1;
}

func main() -> i32 {
  let mut value = 41;
  let r = &mut value;
  bump(r);
  *r
}
"#,
    );
    let output = file.check();
    assert!(
        output.status.success(),
        "exclusive references must be accepted: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn writing_a_field_through_an_exclusive_reference_is_accepted() {
    let file = SourceFile::new(
        "exclusive-field-write",
        r#"struct Point {
  x: i32,
  y: i32,
}

func fill(point: *mut Point) {
  (*point).x = 40;
  (*point).y = 2;
}

func main() -> i32 {
  let mut point = Point { x: 0, y: 0 };
  fill(&mut point);
  point.x + point.y
}
"#,
    );
    let output = file.check();
    assert!(
        output.status.success(),
        "exclusive field writes must be accepted: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn writing_a_field_through_a_shared_reference_reports_e3014() {
    reject(
        "shared-field-write",
        r#"struct Point {
  x: i32,
}

func main() -> i32 {
  let point = Point { x: 1 };
  let r = &point;
  (*r).x = 2;
  (*r).x
}
"#,
        "E3014",
        "shared reference",
    );
}

#[test]
fn borrowing_through_a_reference_value_is_accepted() {
    let file = SourceFile::new(
        "borrow-through-value",
        r#"func identity(p: *i32) -> *i32 {
  p
}

func main() -> i32 {
  let x = 41;
  let r = &*identity(&x);
  *r + 1
}
"#,
    );
    let output = file.check();
    assert!(
        output.status.success(),
        "a borrow through a reference value must type check: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn passing_the_same_exclusive_pointer_twice_reports_e5010() {
    reject(
        "alias-exclusive-arguments",
        r#"func take(a: *mut i32, b: *mut i32) {}

func main() -> i32 {
  let mut x = 1;
  let r = &mut x;
  take(r, r);
  0
}
"#,
        "E5010",
        "arguments 1 and 2 both name `r`",
    );
}

#[test]
fn passing_a_borrowed_place_as_exclusive_reports_e5009() {
    reject(
        "exclusive-argument-borrowed",
        r#"func take(p: *mut i32) {}

func main() -> i32 {
  let mut x = 1;
  let r = &mut x;
  let s = &*r;
  take(r);
  *s
}
"#,
        "E5009",
        "because it is borrowed",
    );
}

#[test]
fn a_non_copy_field_reports_e3077_with_its_span_and_help() {
    let output = reject(
        "copy-field",
        r#"struct Token {
  value: i32,
}

struct Box {
  value: Token,
}

impl Copy for Box {}
"#,
        "E3077",
        "field `value` of `Box` has type `Token`",
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--> "),
        "missing a source location: {stderr}"
    );
    assert!(
        stderr.contains("impl Copy for Box {}") && stderr.contains('^'),
        "missing the implementation span: {stderr}"
    );
    assert!(
        stderr.contains("help:") && stderr.contains("make its type `Copy`"),
        "missing the field help: {stderr}"
    );
}

#[test]
fn copy_and_drop_report_e3068_with_a_secondary_span() {
    let output = reject(
        "copy-drop-conflict",
        r#"struct Point {
  x: i32,
}

impl Copy for Point {}

impl Drop for Point {
  func drop(self: *mut Point) {}
}
"#,
        "E3068",
        "cannot implement `Copy`",
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--> "),
        "missing a source location: {stderr}"
    );
    assert!(
        stderr.contains("impl Copy for Point {}") && stderr.contains('^'),
        "missing the Copy implementation span: {stderr}"
    );
    assert!(
        stderr.contains("the destructor is declared here")
            && stderr.contains("impl Drop for Point"),
        "missing the Drop secondary span: {stderr}"
    );
    assert!(
        stderr.contains("help:") && stderr.contains("cannot own a destructor"),
        "missing the conflict help: {stderr}"
    );
}

#[test]
fn two_shared_arguments_may_alias() {
    let file = SourceFile::new(
        "alias-shared-arguments",
        r#"func sum(a: *i32, b: *i32) -> i32 {
  *a + *b
}

func main() -> i32 {
  let x = 21;
  let r = &x;
  sum(r, r)
}
"#,
    );
    let output = file.check();
    assert!(
        output.status.success(),
        "aliasing shared arguments must be accepted: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
