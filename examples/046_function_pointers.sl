// Seal function pointers have the internal ABI and are one `Copy` word. A
// function item coerces at an expected `func` type, `choose` returns one, and
// the pointer can be stored in a struct, a tuple, or an array. An `unsafe func`
// keeps its caller contract in the type and is called only inside `unsafe`.
// The documented result is 42.

func add(left: i32, right: i32) -> i32 {
  left + right
}

func add_one(value: i32) -> i32 {
  value + 1
}

func double(value: i32) -> i32 {
  value * 2
}

unsafe func unchecked(value: i32) -> i32 {
  value + 1
}

func choose(flag: bool) -> func(i32) -> i32 {
  if flag { add_one } else { double }
}

struct Ops {
  run: func(i32) -> i32,
}

func main() -> i32 {
  let direct: func(i32, i32) -> i32 = add;
  let selected = choose(true);
  let ops = Ops { run: selected };
  let pair = (ops.run, 20);
  let array = [double, add_one];
  let unsafe_pointer: unsafe func(i32) -> i32 = unchecked;

  let stored = array[0](10) + array[1](10);
  if stored == 31 && unsafe { unsafe_pointer(1) } == 2 {
    direct(20, 1) + (pair.0)(pair.1)
  } else {
    0
  }
}
