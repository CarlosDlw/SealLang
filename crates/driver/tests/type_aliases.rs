//! End-to-end coverage for transparent type aliases.

mod support;

use support::{SourceFile, reject};

#[test]
fn aliases_check_through_typeck_borrowck_and_monomorphization() {
    let file = SourceFile::new(
        "type-aliases",
        r#"type Score = i32;
type Pair<T> = (T, T);

struct Point {
  x: Score,
  y: Score,
}

type P = Point;

struct Holder<T> {
  value: T,
}

type IntHolder = Holder<Score>;

type HolderAlias<T> = Holder<T>;

enum Maybe {
  Some(Score),
  None,
}

type M = Maybe;

impl P {
  func origin() -> P {
    P { x: 0, y: 0 }
  }
}

func main() -> i32 {
  let origin = P::origin();
  let point = P { x: 20, y: 22 };
  let pair: Pair<Score> = (origin.x, point.y);
  let holder = IntHolder { value: pair.0 };
  let generic = HolderAlias { value: pair.1 };
  match M::Some(holder.value + generic.value) {
    M::Some(total) => total,
    M::None => 1,
  }
}
"#,
    );
    let output = file.check();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "the alias program did not type-check: {stderr}"
    );
    assert!(stderr.is_empty(), "unexpected diagnostic: {stderr}");
}

#[test]
fn a_recursive_alias_is_rejected_at_the_command_line() {
    reject(
        "recursive-alias",
        "type A = B;\ntype B = A;\nfunc main() {}\n",
        "E2095",
        "expands recursively",
    );
}

#[test]
fn a_bound_on_an_alias_is_rejected_at_the_command_line() {
    reject(
        "alias-bound",
        "type A<T: Copy> = T;\nfunc main() {}\n",
        "E2096",
        "cannot constrain",
    );
}
