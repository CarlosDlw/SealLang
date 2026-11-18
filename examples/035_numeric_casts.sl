// Numeric casts are explicit and infallible. `value as Type` converts only
// between integer and floating-point types; it never panics, and it never
// allocates. Losing information is allowed, which is why the conversion must
// be written out.
//
// The program returns 46 after checking every family of conversion.

func main() -> i32 {
  // Integer to integer: narrowing keeps the low bits, and widening extends
  // with the sign of the source.
  let truncated = 300 as u8; // 300 does not fit, so only the low 8 bits remain
  if truncated != 44u8 { return 1; }
  let extended = truncated as i64;
  if extended != 44i64 { return 2; }

  // Signedness only decides how a widening conversion extends the value; a
  // same-width conversion preserves the bit pattern.
  let bits = -1 as u32;
  if bits != 4294967295u32 { return 3; }
  let wrapped = bits as i32;
  if wrapped != -1 { return 4; }

  // Integer to float rounds to the nearest representable value.
  let scaled = 7 as f64;
  if scaled != 7.0 { return 5; }

  // Float to integer truncates toward zero. NaN becomes zero, and values
  // beyond the target range saturate instead of invoking undefined behavior.
  let rounded = 3.9 as i32;
  if rounded != 3 { return 6; }
  let negative = -3.9 as i32;
  if negative != -3 { return 7; }
  let capped = 1.0e300 as i32;
  if capped != 2147483647 { return 8; }
  let floored = -1.0e300 as i32;
  if floored != -2147483648 { return 9; }
  let nan = (0.0 / 0.0) as i32;
  if nan != 0 { return 10; }

  // Float to float widens exactly and rounds when narrowing.
  let wide = 1.5f32 as f64;
  if wide != 1.5 { return 11; }

  // Casts chain left to right, so each step uses the previous target type.
  let chained = 200 as i8 as i64;
  if chained != -56i64 { return 12; }

  extended as i32 + wrapped + rounded
}
