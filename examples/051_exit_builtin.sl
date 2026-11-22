// `exit(code)` terminates the process with the supplied `i32` status and does
// not return. It performs no implicit cleanup: run destructors with `drop`
// before exiting when their effects matter.
// The documented result is 7.

func main() -> i32 {
  let status = 7;
  exit(status)
}
