// A `static mut` is the only mutable global form. Reading, writing, borrowing,
// and projecting it require an `unsafe` block, and the global effect analysis
// records which functions may touch it, including through calls. A read still
// copies the value, so a mutable global must be `Copy`.
// The documented result is 42.

static mut COUNTER: i32 = 0;
static mut TABLE: [i32; 3] = [1, 2, 3];
static BASE: i32 = 3;

func bump() {
  unsafe {
    COUNTER = COUNTER + 1;
  }
}

// Writing a global makes this function an effect root, and the call to `bump`
// adds that function's effect as well.
func reset() {
  unsafe {
    COUNTER = 0;
  }
  bump();
}

// A block-scope static is one allocation shared by every call.
func next_id() -> i32 {
  static mut NEXT: i32 = 0;
  unsafe {
    NEXT = NEXT + 1;
    NEXT
  }
}

func main() -> i32 {
  // One reset plus one bump leaves the counter at one.
  reset();

  let mut step = 0;
  while step < 4 {
    bump();
    step = step + 1;
  }

  // A write through a global projection needs the same gate.
  unsafe {
    TABLE[1] = 28;
  }

  // Every call advances the shared counter, so the three ids are 1, 2, and 3.
  let from_ids = next_id() + next_id() + next_id();

  let from_counter = unsafe { COUNTER };
  let from_table = unsafe { TABLE[1] };

  from_counter + from_ids + from_table + BASE
}
