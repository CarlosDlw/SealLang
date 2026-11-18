//! Borrow checking through custom indexing protocols.

use super::support::{check_clean, check_errors};

const GRID_MUT: &str = "\
struct Grid { value: i32 } \
impl Index<usize, i32> for Grid { \
    func index(self: *Grid, index: usize) -> *i32 { &(*self).value } \
} \
impl IndexMut<usize, i32> for Grid { \
    func index_mut(self: *mut Grid, index: usize) -> *mut i32 { &mut (*self).value } \
}";

#[test]
fn an_exclusive_index_write_conflicts_with_a_live_shared_borrow() {
    let source = format!(
        "{GRID_MUT} func f() -> i32 {{ \
             let mut grid = Grid {{ value: 0 }}; \
             let view = &grid; \
             grid[0] = 1; \
             (*view).value \
         }}"
    );
    let fixture = check_errors(&source);
    fixture.expect("E5008", "already borrowed");
}

#[test]
fn a_shared_index_read_conflicts_with_a_live_exclusive_borrow() {
    let source = format!(
        "{GRID_MUT} func f() -> i32 {{ \
             let mut grid = Grid {{ value: 0 }}; \
             let view = &mut grid; \
             let value = grid[0]; \
             value + (*view).value \
         }}"
    );
    let fixture = check_errors(&source);
    fixture.expect("E5008", "already borrowed");
}

#[test]
fn an_exclusive_index_borrow_keeps_the_base_borrowed() {
    let source = format!(
        "{GRID_MUT} func f() -> i32 {{ \
             let mut grid = Grid {{ value: 0 }}; \
             let value = &mut grid[0]; \
             let read = grid.value; \
             *value = 1; \
             read \
         }}"
    );
    let fixture = check_errors(&source);
    fixture.expect("E5007", "exclusively borrowed");
}

#[test]
fn a_simple_assignment_can_read_the_same_container() {
    let source = format!(
        "{GRID_MUT} func main() -> i32 {{ \
             let mut grid = Grid {{ value: 1 }}; \
             grid[0] = grid[0] + 1; \
             grid[0] \
         }}"
    );
    check_clean(&source);
}

#[test]
fn indexing_after_the_base_borrow_ends_is_allowed() {
    let source = format!(
        "{GRID_MUT} func f() -> i32 {{ \
             let mut grid = Grid {{ value: 0 }}; \
             let view = &grid; \
             let read = (*view).value; \
             grid[0] = 1; \
             read \
         }}"
    );
    check_clean(&source);
}
