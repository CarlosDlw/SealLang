// Runtime panics are deterministic. This program reports the source location
// of the division and exits with status 101.
func main() -> i32 {
  let divisor = 0;
  10 / divisor
}
