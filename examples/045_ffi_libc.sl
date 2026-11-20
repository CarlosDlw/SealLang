// Seal speaks to the platform C library without a header file or a binding
// generator. `abs` and `snprintf` are ordinary declarations; the compiler
// checks their types, emits the C ABI, and requires each call to be unsafe.

extern "c" {
  func abs(value: c_int) -> c_int;
  func snprintf(buffer: *raw mut u8, size: usize, format: *raw u8, ...) -> c_int;
}

func main() -> i32 {
  let mut buffer: [u8; 16] = [0u8; 16];

  unsafe {
    let value = abs(-7);
    if value != 7 { return 1; }

    // `%d` is a NUL-terminated C string. The variadic `value` is promoted to
    // `c_int` before the call, and `snprintf` writes at most 16 bytes.
    let written = snprintf(
      &mut buffer[0] as *raw mut u8,
      16usize,
      c"value=%d",
      value,
    );
    if written != 7 { return 2; }
    if buffer[0] != 118 { return 3; } // 'v'
    if buffer[5] != 61 { return 4; }  // '='
    if buffer[6] != 55 { return 5; }  // '7'
    if buffer[7] != 0 { return 6; }   // NUL terminator
  }

  7
}
