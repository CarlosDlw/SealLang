// The std prelude puts `Option` and `Result` in scope. Variants stay qualified.

use std::alloc::Layout;
use std::alloc::RawBuffer;

func option_or(value: Option<i32>, fallback: i32) -> i32 {
  match value {
    Option::Some(inner) => inner,
    Option::None => fallback,
  }
}

func result_or(value: Result<i32, i32>, fallback: i32) -> i32 {
  match value {
    Result::Ok(inner) => inner,
    Result::Err(code) => fallback + code,
  }
}

func main() -> i32 {
  let some = Option::Some(41);
  let none = Option::<i32>::None;
  let ok = Result::<i32, i32>::Ok(1);
  let err = Result::<i32, i32>::Err(0);

  let choice = option_or(some, 0) + option_or(none, 0);
  let outcome = result_or(ok, 0) + result_or(err, 0);
  if choice + outcome != 42 {
    return 1;
  }

  // `core::str` is the canonical module behind the `std::str` re-export.
  // `from_utf8` validates external bytes and returns a borrowing view.
  let text: *str = "a🦀é";
  let bytes: [u8; 7] = [97u8, 240u8, 159u8, 166u8, 128u8, 195u8, 169u8];
  let checked: *str = match core::str::from_utf8(&bytes[..]) {
    Option::Some(value) => value,
    Option::None => {
      return 2;
    }
  };
  if checked != text {
    return 3;
  }

  let mut scalars = 0;
  for value in checked {
    let _ = value;
    scalars += 1;
  }
  let mut byte_count = 0;
  for value in checked.bytes() {
    let _ = value;
    byte_count += 1;
  }
  if scalars != 3 || byte_count != 7 {
    return 4;
  }

  match checked.find("🦀") {
    Option::Some(index) => {
      if index != 1 {
        return 5;
      }
    }
    Option::None => {
      return 6;
    }
  }
  if !checked.starts_with("a")
    || !checked.ends_with("é")
    || !checked.contains("🦀")
  {
    return 7;
  }
  if !"HeLLo".eq_ignore_ascii_case("hello") {
    return 8;
  }

  let mut words = 0;
  for word in " one\ttwo ".trim().split_whitespace() {
    let _ = word;
    words += 1;
  }
  let mut pieces = 0;
  for piece in "a,,b,".split(",") {
    let _ = piece;
    pieces += 1;
  }
  let mut lines = 0;
  for line in "a\r\nb\n".lines() {
    let _ = line;
    lines += 1;
  }
  if words != 2 || pieces != 4 || lines != 2 {
    return 9;
  }

  // Allocation is explicit and hosted-only. `RawBuffer` owns the block and
  // releases it on drop, including every early return below.
  let layout = match Layout::from_size_align(16, 8) {
    Result::Ok(layout) => layout,
    Result::Err(_) => {
      return 10;
    }
  };
  let mut buffer = match RawBuffer::new(layout) {
    Result::Ok(buffer) => buffer,
    Result::Err(_) => {
      return 11;
    }
  };

  unsafe { *buffer.as_ptr() = 123; }
  match buffer.resize(32) {
    Result::Ok(_) => {}
    Result::Err(_) => {
      return 12;
    }
  }
  if buffer.capacity() != 32 || buffer.layout().align() != 8 {
    return 13;
  }
  unsafe {
    if *buffer.as_ptr() != 123 {
      return 14;
    }
    *(buffer.as_ptr().add(31)) = 234;
    if *(buffer.as_ptr().add(31)) != 234 {
      return 15;
    }
  }
  drop(buffer);

  0
}
