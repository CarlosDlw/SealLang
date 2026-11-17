// Intrinsic methods are compiler-owned. They are resolved statically, and
// modifying methods borrow their receiver for the duration of the call.

func main() -> i32 {
  let mut total = 0;

  // Integer methods preserve the receiver type.
  total = total + (-7).abs();
  total = total + (-3).signum();
  total = total + 7.min(4);
  total = total + 7.max(10);
  total = total + 7.clamp(8, 9);
  total = total + (-5).abs().min(10);

  if 250u8.wrapping_add(10u8) == 4u8 {
    total = total + 1;
  }
  if 3u8.wrapping_sub(5u8) == 254u8 {
    total = total + 1;
  }
  if 3u8.wrapping_mul(4u8) == 12u8 {
    total = total + 1;
  }
  if 100i32.saturating_add(50) == 150i32 {
    total = total + 1;
  }
  if (-100i32).saturating_sub(50) == -150i32 {
    total = total + 1;
  }
  if 1000i32.saturating_mul(1000) == 1000000i32 {
    total = total + 1;
  }
  if 11i32.count_ones() == 3u32 {
    total = total + 1;
  }
  if 0i32.leading_zeros() == 32u32 {
    total = total + 1;
  }
  if 8i32.trailing_zeros() == 3u32 {
    total = total + 1;
  }
  if 1i32.rotate_left(1) == 2i32 {
    total = total + 1;
  }
  if (-128i8).swap_bytes() == -128i8 {
    total = total + 1;
  }

  // Floating-point methods return the receiver's width.
  let mut float_points = 0;
  if 2.0.sqrt() > 1.4 {
    float_points = float_points + 1;
  }
  if (-3.5).abs() == 3.5 {
    float_points = float_points + 1;
  }
  if 3.7.floor() == 3.0 {
    float_points = float_points + 1;
  }
  if 3.2.ceil() == 4.0 {
    float_points = float_points + 1;
  }
  if 3.5.round() == 4.0 {
    float_points = float_points + 1;
  }
  if (-3.7).trunc() == -3.0 {
    float_points = float_points + 1;
  }
  if 3.25.fract() > 0.24 {
    float_points = float_points + 1;
  }
  if 2.0.powi(3) == 8.0 {
    float_points = float_points + 1;
  }
  if 1.0.min(2.0) == 1.0 {
    float_points = float_points + 1;
  }
  if 1.0.max(2.0) == 2.0 {
    float_points = float_points + 1;
  }
  if 5.0.clamp(1.0, 3.0) == 3.0 {
    float_points = float_points + 1;
  }
  let nan = 0.0 / 0.0;
  if nan.is_nan() && !nan.is_finite() {
    float_points = float_points + 1;
  }
  let infinity = 1.0 / 0.0;
  if infinity.is_infinite() {
    float_points = float_points + 1;
  }
  let finite = 4.0;
  if finite.is_finite() {
    float_points = float_points + 1;
  }
  total = total + float_points;

  // Boolean methods either read or modify the receiver.
  let mut flag = false;
  if flag.not() && flag.to_i32() == 0 {
    total = total + 1;
  }
  flag.toggle();
  if flag && flag.to_i32() == 1 {
    total = total + 1;
  }

  // Character methods inspect Unicode scalar values.
  let letter = 'A';
  if letter.is_ascii() && letter.is_ascii_alphabetic() && !letter.is_ascii_digit() {
    total = total + 1;
  }
  if letter.to_ascii_lowercase() == 'a' {
    total = total + 1;
  }
  if letter.to_u32() == 65u32 && letter.len_utf8() == 1usize {
    total = total + 1;
  }
  if ' '.is_ascii_whitespace() && '0'.is_ascii_digit() {
    total = total + 1;
  }

  // Arrays offer the same sequence methods as slices.
  let mut numbers = [1, 2, 3, 4];
  if numbers.len() == 4usize && !numbers.is_empty() {
    total = total + 1;
  }
  numbers.swap(0, 3);
  numbers.reverse();
  numbers.fill(7);
  if numbers[0] == 7 && numbers[3] == 7 {
    total = total + 1;
  }

  // A slice method receives the fat pointer and uses its runtime length.
  let view: *[i32] = &numbers;
  if view.len() == 4usize && !view.is_empty() {
    total = total + 1;
  }
  total = total + view[2];

  // An exclusive slice can reorder and overwrite the borrowed storage.
  let mut data = [1, 2, 3];
  let mut window: *mut [i32] = &mut data[..];
  window.swap(0, 2);
  window.reverse();
  window.fill(5);
  total = total + data[0] + data[1] + data[2];

  total
}
