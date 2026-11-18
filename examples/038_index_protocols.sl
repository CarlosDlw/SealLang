// `Index<Key, Value>` returns a shared reference to the selected element and
// `IndexMut<Key, Value>` returns an exclusive one. Reads use `Index`; writes,
// compound assignments, and exclusive borrows use `IndexMut`. Indexing nests
// like built-in array indexing, and each protocol call is an ordinary call.
// The documented result is 42.

struct Cell {
  value: i32,
}

impl Index<usize, i32> for Cell {
  func index(self: *Cell, index: usize) -> *i32 {
    &(*self).value
  }
}

impl IndexMut<usize, i32> for Cell {
  func index_mut(self: *mut Cell, index: usize) -> *mut i32 {
    &mut (*self).value
  }
}

struct Row {
  left: Cell,
  right: Cell,
}

impl Index<usize, Cell> for Row {
  func index(self: *Row, index: usize) -> *Cell {
    if index == 0 {
      &(*self).left
    } else {
      &(*self).right
    }
  }
}

impl IndexMut<usize, Cell> for Row {
  func index_mut(self: *mut Row, index: usize) -> *mut Cell {
    if index == 0 {
      &mut (*self).left
    } else {
      &mut (*self).right
    }
  }
}

func main() -> i32 {
  let mut row = Row {
    left: Cell { value: 10 },
    right: Cell { value: 20 },
  };

  // The outer write selects Row's IndexMut, then Cell's IndexMut.
  row[0][0] += 5;
  row[1][0] += 7;

  // Reads at both levels select Index.
  row[0][0] + row[1][0]
}
