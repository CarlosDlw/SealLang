//! End-to-end code generation for custom iteration and indexing protocols.

use seal_codegen::Emit;
use seal_session::OptLevel;

mod support;

use support::{TempOutput, compile, run};

/// Compiles `source` at `O0` and `O2` and asserts the executable's exit code.
fn assert_result(source: &str, expected: i32, stem: &str) {
    for (opt_level, debug_info, suffix) in [
        (OptLevel::O0, true, format!("{stem}-debug")),
        (OptLevel::O2, false, format!("{stem}-release")),
    ] {
        let output = TempOutput::new(&suffix);
        compile(
            source,
            output.path(),
            Emit::Executable,
            opt_level,
            debug_info,
        )
        .unwrap_or_else(|error| panic!("{stem} failed to compile at {opt_level:?}: {error}"));
        assert_eq!(
            run(output.path()).code(),
            Some(expected),
            "{stem} produced the wrong result at {opt_level:?}"
        );
    }
}

const ITERATION: &str = "\
struct Source { value: i32 } \
struct Cursor { value: i32 } \
impl Iterator<i32> for Cursor { \
    func has_next(self: *Cursor) -> bool { (*self).value < 43 } \
    func next(self: *mut Cursor) -> i32 { \
        let value = (*self).value; \
        (*self).value += 1; \
        value \
    } \
} \
impl IntoIterator<Cursor, i32> for Source { \
    func iter(self: Source) -> Cursor { Cursor { value: self.value } } \
} \
func main() -> i32 { \
    let source = Source { value: 42 }; \
    let mut result = 0; \
    for value in source { result = value; } \
    result \
}";

const INDEXING: &str = "\
struct Grid { value: i32 } \
impl Index<usize, i32> for Grid { \
    func index(self: *Grid, index: usize) -> *i32 { &(*self).value } \
} \
impl IndexMut<usize, i32> for Grid { \
    func index_mut(self: *mut Grid, index: usize) -> *mut i32 { &mut (*self).value } \
} \
func main() -> i32 { \
    let mut grid = Grid { value: 40 }; \
    grid[0] += 1; \
    grid[0] = 42; \
    grid[0] \
}";

const DYNAMIC_INDEXING: &str = "\
struct Grid { value: i32 } \
impl Index<usize, i32> for Grid { \
    func index(self: *Grid, index: usize) -> *i32 { &(*self).value } \
} \
impl IndexMut<usize, i32> for Grid { \
    func index_mut(self: *mut Grid, index: usize) -> *mut i32 { &mut (*self).value } \
} \
func read(grid: *dyn Index<usize, i32>) -> i32 { grid[0] } \
func write(grid: *mut dyn IndexMut<usize, i32>, value: i32) { grid[0] = value; } \
func main() -> i32 { \
    let mut grid = Grid { value: 40 }; \
    let shared: *dyn Index<usize, i32> = &grid; \
    let before = read(shared); \
    let exclusive: *mut dyn IndexMut<usize, i32> = &mut grid; \
    write(exclusive, before + 2); \
    let shared_again: *dyn Index<usize, i32> = &grid; \
    read(shared_again) \
}";

#[test]
fn the_iteration_protocol_runs_at_o0_and_o2() {
    assert_result(ITERATION, 42, "iteration-protocol");
}

#[test]
fn the_index_protocol_runs_at_o0_and_o2() {
    assert_result(INDEXING, 42, "index-protocol");
}

#[test]
fn dynamic_indexing_runs_at_o0_and_o2() {
    assert_result(DYNAMIC_INDEXING, 42, "dynamic-index-protocol");
}
