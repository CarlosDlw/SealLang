//! End-to-end regression tests for the diagnostics users see on the command
//! line.
//!
//! Every case is a short program that must be rejected with a stable code, a
//! message fragment, and exit status 1. A diagnostic that regresses to an
//! internal compiler error, changes its code, or silently accepts the program
//! fails here instead of reaching a user.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT_FILE: AtomicU32 = AtomicU32::new(0);

struct SourceFile(PathBuf);

impl SourceFile {
    fn new(name: &str, source: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "seal-diagnostics-{}-{}-{name}.sl",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, source).expect("the test source is writable");
        Self(path)
    }

    fn check(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_sealc"))
            .arg("--emit=check")
            .arg(&self.0)
            .output()
            .expect("the compiler runs")
    }
}

impl Drop for SourceFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Runs `source` and requires a compilation error with `code` and `fragment`.
fn reject(name: &str, source: &str, code: &str, fragment: &str) {
    let file = SourceFile::new(name, source);
    let output = file.check();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{name}: expected exit status 1, got {:?}: {stderr}",
        output.status
    );
    assert!(
        stderr.contains(&format!("error[{code}]")),
        "{name}: expected {code}, got: {stderr}"
    );
    assert!(
        stderr.contains(fragment),
        "{name}: expected {fragment:?} in: {stderr}"
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
