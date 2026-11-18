//! Command-line regression tests for iteration and indexing diagnostics.

mod support;

use support::reject;

#[test]
fn redeclaring_a_protocol_trait_reports_e2066() {
    reject(
        "reserved-protocol",
        "trait Iterator { }\nfunc main() { }\n",
        "E2066",
        "reserved",
    );
}

#[test]
fn implementing_a_protocol_for_an_array_reports_e2067() {
    reject(
        "protocol-array",
        "impl Index<usize, i32> for [i32; 2] { }\nfunc main() { }\n",
        "E2067",
        "compiler-owned type",
    );
}

#[test]
fn two_into_iterator_impls_report_e3050() {
    reject(
        "ambiguous-into-iterator",
        r#"struct Source { value: i32 }
struct First { value: i32 }
struct Second { value: i32 }
impl Iterator<i32> for First {
  func has_next(self: *First) -> bool { true }
  func next(self: *mut First) -> i32 { 0 }
}
impl Iterator<i32> for Second {
  func has_next(self: *Second) -> bool { true }
  func next(self: *mut Second) -> i32 { 0 }
}
impl IntoIterator<First, i32> for Source {
  func iter(self: Source) -> First { First { value: 0 } }
}
impl IntoIterator<Second, i32> for Source {
  func iter(self: Source) -> Second { Second { value: 0 } }
}
func main() {
  let source = Source { value: 1 };
  for value in source { let _ = value; }
}
"#,
        "E3050",
        "cannot choose an `IntoIterator` implementation",
    );
}

#[test]
fn an_iterator_without_the_iterator_protocol_reports_e3051() {
    reject(
        "missing-iterator",
        r#"struct Source { value: i32 }
struct Cursor { value: i32 }
impl IntoIterator<Cursor, i32> for Source {
  func iter(self: Source) -> Cursor { Cursor { value: 0 } }
}
func main() {
  let source = Source { value: 1 };
  for value in source { let _ = value; }
}
"#,
        "E3051",
        "does not implement `Iterator<i32>`",
    );
}

#[test]
fn two_iterator_impls_report_e3052() {
    reject(
        "ambiguous-iterator",
        r#"struct Source { value: i32 }
impl Iterator<i32> for Source {
  func has_next(self: *Source) -> bool { true }
  func next(self: *mut Source) -> i32 { 0 }
}
impl Iterator<bool> for Source {
  func has_next(self: *Source) -> bool { true }
  func next(self: *mut Source) -> bool { false }
}
func main() {
  let source = Source { value: 1 };
  for value in source { let _ = value; }
}
"#,
        "E3052",
        "cannot choose an `Iterator` implementation",
    );
}

#[test]
fn two_index_impls_report_e3053() {
    reject(
        "ambiguous-index",
        r#"struct Grid { value: i32 }
impl Index<usize, i32> for Grid {
  func index(self: *Grid, index: usize) -> *i32 { &(*self).value }
}
impl Index<bool, i32> for Grid {
  func index(self: *Grid, index: bool) -> *i32 { &(*self).value }
}
func main() -> i32 {
  let grid = Grid { value: 7 };
  grid[0]
}
"#,
        "E3053",
        "cannot choose an `Index` implementation",
    );
}

#[test]
fn writing_through_an_index_only_reports_e3054() {
    reject(
        "index-not-mutable",
        r#"struct Grid { value: i32 }
impl Index<usize, i32> for Grid {
  func index(self: *Grid, index: usize) -> *i32 { &(*self).value }
}
func main() {
  let grid = Grid { value: 1 };
  grid[0] = 2;
}
"#,
        "E3054",
        "cannot write through an index",
    );
}

#[test]
fn an_unconstrained_index_element_reports_e3055() {
    reject(
        "unconstrained-index",
        r#"struct Grid { value: i32 }
impl<I, T> Index<I, T> for Grid {
  func index(self: *Grid, index: I) -> *T { self.index(index) }
}
func main() {
  let grid = Grid { value: 1 };
  let _ = grid[0];
}
"#,
        "E3055",
        "cannot infer",
    );
}
