// The introspection built-ins report static type information without
// allocating or formatting at run time. `type_name` and `type_of` produce the
// same source spelling used in diagnostics; `size_of` and `align_of` use the
// target layout.
// The documented result is 6.

func main() -> i32 {
  let mut score = 0;
  if type_name::<i32>() == "i32" {
    score += 1;
  }
  if type_name::<[u8; 4]>() == "[u8; 4]" {
    score += 1;
  }
  if type_of(score) == "i32" {
    score += 1;
  }
  if type_of(&score) == "*i32" {
    score += 1;
  }
  if size_of::<i32>() == 4 {
    score += 1;
  }
  if align_of::<u64>() == 8 {
    score += 1;
  }
  score
}
